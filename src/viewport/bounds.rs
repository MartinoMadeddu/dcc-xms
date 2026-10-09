//! Bounding boxes for composed stages, as Gaffer's viewer draws them.
//!
//! A prim closed in the scene explorer is drawn as the box around everything
//! below it; opening it hands the decision to its children. A prim with no
//! children (a mesh) is drawn as geometry once its parent is open, and so is
//! everything below a prim whose geometry toggle is on. "Show all geometry"
//! draws everything. So the island first shows one box per root prim, and
//! only what is opened costs anything to draw.
//!
//! The boxes come from the packed primitives as they arrive at the viewport,
//! not from the file: a Transform or Prune upstream moves or removes them.
//! A point instancer's box covers every copy.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};

use bevy::math::{Mat4, Vec3};

use crate::types::{MeshData, NamedMesh, SceneHierarchy};
use crate::usd_scene::StageTree;

/// An axis-aligned box: low and high corners.
pub type Bounds = (Vec3, Vec3);

fn union(a: Option<Bounds>, b: Bounds) -> Option<Bounds> {
    Some(match a { Some(a) => (a.0.min(b.0), a.1.max(b.1)), None => b })
}

/// The box of a mesh in its own space. Remembered per shared mesh, so a
/// heavy model is measured once.
fn mesh_bounds(mesh: &Arc<MeshData>) -> Option<Bounds> {
    type Kept = HashMap<usize, (Weak<MeshData>, Option<Bounds>)>;
    static KEPT: Mutex<Option<Kept>> = Mutex::new(None);
    let key = Arc::as_ptr(mesh) as usize;
    let mut kept = KEPT.lock().unwrap();
    let kept = kept.get_or_insert_with(HashMap::new);
    if let Some((weak, b)) = kept.get(&key) {
        if weak.upgrade().is_some_and(|m| Arc::ptr_eq(&m, mesh)) { return *b; }
    }
    let b = mesh.vertices.iter().chain(&mesh.points).chain(&mesh.curve_points)
        .map(|v| Vec3::from_array(*v))
        .fold(None, |acc, v| union(acc, (v, v)));
    if kept.len() > 8192 { kept.retain(|_, (w, _)| w.strong_count() > 0); }
    kept.insert(key, (Arc::downgrade(mesh), b));
    b
}

/// A box moved by a matrix: the box around the moved box.
fn moved(b: Bounds, m: &Mat4) -> Bounds {
    let centre = m.transform_point3((b.0 + b.1) * 0.5);
    let half = (b.1 - b.0) * 0.5;
    let reach = Vec3::new(
        m.x_axis.x.abs() * half.x + m.y_axis.x.abs() * half.y + m.z_axis.x.abs() * half.z,
        m.x_axis.y.abs() * half.x + m.y_axis.y.abs() * half.y + m.z_axis.y.abs() * half.z,
        m.x_axis.z.abs() * half.x + m.y_axis.z.abs() * half.y + m.z_axis.z.abs() * half.z,
    );
    (centre - reach, centre + reach)
}

/// Where a packed primitive is drawn, every copy included.
pub fn world_bounds(p: &NamedMesh) -> Option<Bounds> {
    let local = mesh_bounds(&p.mesh)?;
    if !p.place.is_placed() { return Some(local); }
    p.place.matrices().iter().fold(None, |acc, m| union(acc, moved(local, m)))
}

/// Split packed primitives into those drawn as geometry and the boxes drawn
/// in place of the others.
pub fn split(prims: Vec<NamedMesh>, hierarchy: &SceneHierarchy) -> (Vec<NamedMesh>, Vec<Bounds>) {
    if hierarchy.show_all_geometry { return (prims, vec![]); }
    let mut trees: Vec<Arc<StageTree>> = vec![];
    for p in &prims {
        if let Some(t) = &p.stage {
            if !t.nodes.is_empty() && !trees.iter().any(|x| Arc::ptr_eq(x, t)) { trees.push(t.clone()); }
        }
    }
    if trees.is_empty() { return (prims, vec![]); }
    let mut boxes = vec![];
    let mut boxed: HashSet<usize> = HashSet::new();   // indices into `prims`
    for tree in &trees {
        let mine: Vec<usize> = (0..prims.len())
            .filter(|&i| prims[i].stage.as_ref().is_some_and(|t| Arc::ptr_eq(t, tree)))
            .collect();
        decide(tree, &prims, &mine, hierarchy, &mut boxes, &mut boxed);
    }
    let geometry = prims.into_iter().enumerate().filter(|(i, _)| !boxed.contains(i)).map(|(_, p)| p).collect();
    (geometry, boxes)
}

/// One stage: walk its hierarchy from the top, as the explorer shows it.
fn decide(
    tree:      &StageTree,
    prims:     &[NamedMesh],
    mine:      &[usize],
    hierarchy: &SceneHierarchy,
    boxes:     &mut Vec<Bounds>,
    boxed:     &mut HashSet<usize>,
) {
    let nodes = &tree.nodes;
    let n = nodes.len();
    let index: HashMap<&str, usize> = nodes.iter().enumerate().map(|(i, node)| (node.path.as_str(), i)).collect();

    // Each primitive belongs to the prim of its path, or else to the nearest
    // listed prim above it.
    let mut owner: Vec<(usize, usize)> = vec![];   // (primitive, node)
    let mut own: Vec<Option<Bounds>> = vec![None; n];
    for &k in mine {
        let mut path = prims[k].path.as_str();
        let node = loop {
            if let Some(&i) = index.get(path) { break Some(i); }
            match path.rfind('/') { Some(0) | None => break None, Some(cut) => path = &path[..cut] }
        };
        let Some(node) = node else { continue };
        owner.push((k, node));
        if let Some(b) = world_bounds(&prims[k]) { own[node] = union(own[node], b); }
    }

    // Parents, the end of each subtree, and the box around each subtree.
    let mut parent = vec![usize::MAX; n];
    let mut end = vec![n; n];
    let mut open: Vec<usize> = vec![];
    for i in 0..n {
        while let Some(&p) = open.last() {
            if nodes[p].depth >= nodes[i].depth { end[p] = i; open.pop(); } else { break; }
        }
        if let Some(&p) = open.last() { parent[i] = p; }
        open.push(i);
    }
    let mut total = own;
    for i in (0..n).rev() {
        if parent[i] != usize::MAX {
            if let Some(b) = total[i] { total[parent[i]] = union(total[parent[i]], b); }
        }
    }

    // Down the hierarchy: open prims pass the decision on, closed ones are
    // a box, leaves and prims with their geometry toggle on are geometry.
    let mut covered = vec![false; n];
    let mut i = 0;
    while i < n {
        let node = &nodes[i];
        let leaf = end[i] == i + 1;
        if leaf || hierarchy.shows_geometry(&node.path) { i = end[i]; continue; }
        if hierarchy.is_expanded(&node.path) { i += 1; continue; }
        if let Some(b) = total[i] { boxes.push(b); }
        for c in covered.iter_mut().take(end[i]).skip(i) { *c = true; }
        i = end[i];
    }
    for (k, node) in owner {
        if covered[node] { boxed.insert(k); }
    }
}

/// Every box as edges, in one line mesh: one draw for all of them.
pub fn box_lines(boxes: &[Bounds]) -> (Vec<[f32; 3]>, Vec<u32>) {
    const EDGES: [[u32; 2]; 12] = [[0, 1], [1, 3], [3, 2], [2, 0], [4, 5], [5, 7], [7, 6], [6, 4], [0, 4], [1, 5], [2, 6], [3, 7]];
    let mut points = Vec::with_capacity(boxes.len() * 8);
    let mut lines = Vec::with_capacity(boxes.len() * 24);
    for (lo, hi) in boxes {
        let base = points.len() as u32;
        for c in 0..8u32 {
            points.push([
                if c & 1 != 0 { hi.x } else { lo.x },
                if c & 2 != 0 { hi.y } else { lo.y },
                if c & 4 != 0 { hi.z } else { lo.z },
            ]);
        }
        for [a, b] in EDGES { lines.extend([base + a, base + b]); }
    }
    (points, lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Placement;
    use crate::usd_scene::StageNode;

    fn node(path: &str, depth: usize, ty: &str) -> StageNode {
        StageNode { path: path.into(), name: path.rsplit('/').next().unwrap().into(), type_name: ty.into(), depth, hidden: false }
    }
    fn cube_at(path: &str, x: f32, tree: &Arc<StageTree>) -> NamedMesh {
        let mesh = crate::node_graph::nodes::transform(&crate::node_graph::nodes::create_cube(1.0), Vec3::new(x, 0.0, 0.0), Vec3::ZERO, Vec3::ONE);
        NamedMesh { stage: Some(tree.clone()), ..NamedMesh::new(path.into(), mesh) }
    }

    #[test]
    fn closed_prims_are_boxes_and_open_ones_hand_down() {
        let tree = Arc::new(StageTree { nodes: vec![
            node("/world", 1, "Xform"),
            node("/world/a", 2, "Mesh"),
            node("/world/grp", 2, "Xform"),
            node("/world/grp/b", 3, "Mesh"),
            node("/world/grp/c", 3, "Mesh"),
        ]});
        let prims = || vec![cube_at("/world/a", 0.0, &tree), cube_at("/world/grp/b", 10.0, &tree), cube_at("/world/grp/c", 20.0, &tree)];
        let mut h = SceneHierarchy::default();

        // Everything closed: one box around the three cubes, no geometry.
        let (geo, boxes) = split(prims(), &h);
        assert!(geo.is_empty());
        assert_eq!(boxes.len(), 1);
        assert!((boxes[0].0.x + 0.5).abs() < 1e-5 && (boxes[0].1.x - 20.5).abs() < 1e-5);

        // /world open: its mesh is geometry, the closed group is a box.
        h.set_expanded("/world", true);
        let (geo, boxes) = split(prims(), &h);
        assert_eq!(geo.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), vec!["/world/a"]);
        assert_eq!(boxes.len(), 1);

        // The group's geometry toggle on: everything is geometry.
        h.toggle_geometry("/world/grp");
        let (geo, boxes) = split(prims(), &h);
        assert_eq!((geo.len(), boxes.len()), (3, 0));

        // Show all geometry overrides it all.
        let mut h = SceneHierarchy::default();
        h.set_show_all_geometry(true);
        assert_eq!(split(prims(), &h).0.len(), 3);
    }

    #[test]
    fn a_point_instancer_box_covers_every_copy() {
        let mesh = Arc::new(crate::node_graph::nodes::create_cube(1.0));
        let place = Placement::Many((0..3).map(|i| Mat4::from_translation(Vec3::new(0.0, i as f32 * 5.0, 0.0))).collect());
        let p = NamedMesh { mesh, place, ..NamedMesh::new("/trees/proto".into(), MeshData::default()) };
        let b = world_bounds(&p).unwrap();
        assert!((b.0.y + 0.5).abs() < 1e-5 && (b.1.y - 10.5).abs() < 1e-5);
    }

    #[test]
    fn a_turned_box_still_holds_its_corners() {
        let m = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_4);
        let b = moved((Vec3::splat(-1.0), Vec3::splat(1.0)), &m);
        assert!((b.1.x - std::f32::consts::SQRT_2).abs() < 1e-5);
    }
}
