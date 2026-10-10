//! Saving and loading the node graph as JSON, copying and pasting nodes,
//! adding one graph to another, and ready-made graphs.
//!
//! The contents of ICE subnets are not saved in a graph file: a loaded
//! Subnet node starts with a fresh, empty subnet. Copied nodes carry their
//! ICE trees.

use std::path::Path;

use bevy_egui::egui;
use serde::{Deserialize, Serialize};

use crate::ice::SubnetStore;
use crate::node_graph::NodeGraphState;
use crate::types::{NodeId, NodeType, SplitPick, SubnetId, DEFAULT_WRITE_PATH};

#[derive(Serialize, Deserialize)]
struct SavedNode {
    id:        usize,
    name:      String,
    position:  [f32; 2],
    node_type: NodeType,
    #[serde(default)]
    bypassed:  bool,
}

#[derive(Serialize, Deserialize)]
struct SavedGraph {
    version:     u32,
    nodes:       Vec<SavedNode>,
    /// from node, from output, to node, to input
    connections: Vec<[usize; 4]>,
    view_flag:   Option<usize>,
}

pub fn to_json(graph: &NodeGraphState) -> String {
    let saved = SavedGraph {
        version: 1,
        nodes: graph.nodes.iter().map(|n| SavedNode {
            id:        n.id.0,
            name:      n.name.clone(),
            position:  [n.position.x, n.position.y],
            node_type: n.node_type.clone(),
            bypassed:  n.bypassed,
        }).collect(),
        connections: graph.connections.iter()
            .map(|c| [c.from_node.0, c.from_output, c.to_node.0, c.to_input])
            .collect(),
        view_flag: graph.view_flag.map(|n| n.0),
    };
    serde_json::to_string_pretty(&saved).unwrap_or_default()
}

/// Replace the contents of `graph` with a saved graph. View settings (pan,
/// zoom) are kept.
pub fn from_json(graph: &mut NodeGraphState, json: &str) -> Result<(), String> {
    let mut saved: SavedGraph = serde_json::from_str(json).map_err(|e| e.to_string())?;
    upgrade(&mut saved);
    if !saved.nodes.iter().any(|n| matches!(n.node_type, NodeType::Output)) {
        return Err("graph has no Output node".into());
    }
    clear(graph);
    graph.nodes.clear();
    for n in saved.nodes {
        let mut node_type = n.node_type;
        if let NodeType::Subnet { id, .. } = &mut node_type { *id = SubnetId(usize::MAX); }
        let id = graph.add_node(n.name, node_type, egui::pos2(n.position[0], n.position[1]));
        // Keep the saved ids so connections can be restored as written.
        if let Some(node) = graph.nodes.iter_mut().find(|x| x.id == id) { node.id = NodeId(n.id); node.bypassed = n.bypassed; }
        graph.next_node_id = graph.next_node_id.max(n.id + 1);
    }
    for [from, out, to, input] in saved.connections {
        let ok = graph.nodes.iter().any(|n| n.id.0 == from && out < n.outputs.len())
              && graph.nodes.iter().any(|n| n.id.0 == to && input < n.inputs.len());
        if ok { graph.add_connection(NodeId(from), out, NodeId(to), input); }
    }
    graph.view_flag = saved.view_flag.map(NodeId).filter(|v| graph.nodes.iter().any(|n| n.id == *v));
    Ok(())
}

/// Bring a graph saved by an older version up to date:
/// - Transform Clip becomes Transform, which now moves clips too. The turn
///   is the same rotation, written in Transform's convention.
/// - Calamari goes. Its display moves onto the Body Collide node it came
///   after, and the wires through it are joined up.
fn upgrade(saved: &mut SavedGraph) {
    use bevy::math::{EulerRot, Quat, Vec3};
    for n in saved.nodes.iter_mut() {
        if let NodeType::TransformClip { translate, rotate, scale } = n.node_type.clone() {
            let q = Quat::from_euler(EulerRot::YXZ, rotate[1].to_radians(), rotate[0].to_radians(), rotate[2].to_radians());
            let (x, y, z) = q.to_euler(EulerRot::XYZ);
            n.node_type = NodeType::Transform { translation: Vec3::from_array(translate), rotation: Vec3::new(x, y, z), scale: Vec3::splat(scale) };
        }
    }
    let calamari: Vec<(usize, bool)> = saved.nodes.iter()
        .filter_map(|n| match n.node_type { NodeType::Calamari { hulls, .. } => Some((n.id, hulls)), _ => None }).collect();
    for (id, hulls) in calamari {
        let from = saved.connections.iter().find(|c| c[2] == id && c[3] == 0).map(|c| (c[0], c[1]));
        let to: Vec<(usize, usize)> = saved.connections.iter().filter(|c| c[0] == id).map(|c| (c[2], c[3])).collect();
        saved.connections.retain(|c| c[0] != id && c[2] != id);
        if let Some((src, out)) = from {
            for (dst, input) in to { saved.connections.push([src, out, dst, input]); }
            if let Some(n) = saved.nodes.iter_mut().find(|n| n.id == src) {
                if let NodeType::Ragdoll { view, .. } = &mut n.node_type {
                    *view = if hulls { crate::types::BodyView::Hulls } else { crate::types::BodyView::Pieces };
                }
            }
            if saved.view_flag == Some(id) { saved.view_flag = Some(src); }
        } else if saved.view_flag == Some(id) {
            saved.view_flag = None;
        }
        saved.nodes.retain(|n| n.id != id);
    }
}

pub fn save(graph: &NodeGraphState, path: &Path) -> Result<(), String> {
    std::fs::write(path, to_json(graph)).map_err(|e| e.to_string())
}

pub fn load(graph: &mut NodeGraphState, path: &Path) -> Result<(), String> {
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    from_json(graph, &json)
}

// ── Copy, paste, and adding one graph to another ─────────────────────────────

/// Nodes copied from a scene network: their parameters, the wires between
/// them and the trees of their ICE nodes. Held as JSON text in the system
/// clipboard and in `clipboard.json` in the config folder, so it can be
/// pasted in another window of the program, or after it was closed.
#[derive(Serialize, Deserialize)]
pub struct Fragment {
    /// Marks the text as XMS nodes: other text in the clipboard is ignored.
    xms_nodes:   u32,
    nodes:       Vec<SavedNode>,
    /// from node, from output, to node, to input
    connections: Vec<[usize; 4]>,
    #[serde(default)]
    trees:       Vec<SavedTree>,
}

/// The tree of an ICE node.
#[derive(Serialize, Deserialize)]
struct SavedTree {
    /// The ICE node it belongs to.
    node:        usize,
    name:        String,
    nodes:       Vec<SavedTreeNode>,
    connections: Vec<[usize; 4]>,
}

#[derive(Serialize, Deserialize)]
struct SavedTreeNode {
    id:        usize,
    name:      String,
    position:  [f32; 2],
    node_type: crate::types::SubnetNodeType,
}

/// What pasting placed: the new id of each node, by the id it was copied with.
pub type Placed = std::collections::HashMap<usize, NodeId>;

impl Fragment {
    pub fn len(&self) -> usize { self.nodes.len() }

    pub fn to_text(&self) -> String { serde_json::to_string_pretty(self).unwrap_or_default() }

    /// The fragment in a text, if the text is one.
    pub fn from_text(text: &str) -> Option<Self> {
        let f: Self = serde_json::from_str(text.trim()).ok()?;
        (f.xms_nodes >= 1 && !f.nodes.is_empty()).then_some(f)
    }
}

/// Copy nodes, with the wires between them. Output is left out.
pub fn copy(graph: &NodeGraphState, subnets: &SubnetStore, ids: &[NodeId]) -> Option<Fragment> {
    let picked: Vec<&crate::node_graph::GraphNode> = graph.nodes.iter()
        .filter(|n| ids.contains(&n.id) && !matches!(n.node_type, NodeType::Output)).collect();
    if picked.is_empty() { return None; }
    let inside = |id: NodeId| picked.iter().any(|n| n.id == id);
    let trees = picked.iter().filter_map(|n| match &n.node_type {
        NodeType::Subnet { id, .. } => subnets.get(*id).map(|sg| SavedTree {
            node: n.id.0,
            name: sg.name.clone(),
            nodes: sg.nodes.iter().map(|t| SavedTreeNode {
                id: t.id.0, name: t.name.clone(), position: [t.position.x, t.position.y], node_type: t.node_type.clone(),
            }).collect(),
            connections: sg.connections.iter().map(|c| [c.from_node.0, c.from_output, c.to_node.0, c.to_input]).collect(),
        }),
        _ => None,
    }).collect();
    Some(Fragment {
        xms_nodes: 1,
        nodes: picked.iter().map(|n| SavedNode {
            id: n.id.0, name: n.name.clone(), position: [n.position.x, n.position.y],
            node_type: n.node_type.clone(), bypassed: n.bypassed,
        }).collect(),
        connections: graph.connections.iter()
            .filter(|c| inside(c.from_node) && inside(c.to_node))
            .map(|c| [c.from_node.0, c.from_output, c.to_node.0, c.to_input]).collect(),
        trees,
    })
}

/// Paste a fragment: new nodes with the same parameters and wires, the
/// top-left one at `at` (or beside where they were copied from). Each ICE
/// node gets a tree of its own. The pasted nodes are selected.
pub fn paste(graph: &mut NodeGraphState, subnets: &mut SubnetStore, fragment: Fragment, at: Option<egui::Pos2>) -> Placed {
    let Fragment { nodes, connections, trees, .. } = fragment;
    // Nodes from an older version are brought up to date as a file is.
    let mut saved = SavedGraph { version: 1, nodes, connections, view_flag: None };
    upgrade(&mut saved);
    let corner = saved.nodes.iter().fold(egui::pos2(f32::MAX, f32::MAX), |c, n| egui::pos2(c.x.min(n.position[0]), c.y.min(n.position[1])));
    let shift = match at { Some(at) => at - corner, None => egui::vec2(40.0, 40.0) };
    let mut placed = Placed::new();
    for n in saved.nodes {
        let mut node_type = n.node_type;
        if let NodeType::Subnet { id, name } = &mut node_type {
            *id = trees.iter().find(|t| t.node == n.id)
                .map(|t| restore_tree(t, name, subnets))
                .unwrap_or(SubnetId(usize::MAX));
        }
        let new = graph.add_node(n.name, node_type, egui::pos2(n.position[0], n.position[1]) + shift);
        if let Some(node) = graph.nodes.iter_mut().find(|x| x.id == new) { node.bypassed = n.bypassed; }
        placed.insert(n.id, new);
    }
    for [from, out, to, input] in saved.connections {
        let (Some(&a), Some(&b)) = (placed.get(&from), placed.get(&to)) else { continue };
        let ok = graph.nodes.iter().any(|n| n.id == a && out < n.outputs.len())
              && graph.nodes.iter().any(|n| n.id == b && input < n.inputs.len());
        if ok { graph.add_connection(a, out, b, input); }
    }
    let mut new: Vec<NodeId> = placed.values().copied().collect();
    new.sort_by_key(|id| id.0);
    graph.selected_node = new.first().copied();
    graph.selected_nodes = new;
    graph.selected_connection = None;
    placed
}

fn restore_tree(t: &SavedTree, name: &str, subnets: &mut SubnetStore) -> SubnetId {
    let sid = subnets.create_subnet(name.to_string());
    let Some(sg) = subnets.get_mut(sid) else { return sid };
    sg.name = t.name.clone();
    sg.nodes.clear();
    sg.connections.clear();
    sg.next_node_id = 0;
    for n in &t.nodes {
        let id = sg.add_node(n.name.clone(), n.node_type.clone(), egui::pos2(n.position[0], n.position[1]));
        if let Some(node) = sg.nodes.iter_mut().find(|x| x.id == id) { node.id = NodeId(n.id); }
        sg.next_node_id = sg.next_node_id.max(n.id + 1);
    }
    for &[from, out, to, input] in &t.connections {
        let ok = sg.nodes.iter().any(|n| n.id.0 == from && out < n.outputs.len())
              && sg.nodes.iter().any(|n| n.id.0 == to && input < n.inputs.len());
        if ok { sg.add_connection(NodeId(from), out, NodeId(to), input); }
    }
    sid
}

/// Add every node of `other` to `graph`, to the right of what is there.
/// When `graph`'s Output is not wired, it takes what `other`'s was wired
/// to; when nothing is viewed, the viewed node of `other` is. Returns how
/// many nodes were added.
pub fn add_graph(graph: &mut NodeGraphState, subnets: &mut SubnetStore, other: &NodeGraphState) -> usize {
    let ids: Vec<NodeId> = other.nodes.iter().map(|n| n.id).collect();
    let Some(fragment) = copy(other, subnets, &ids) else { return 0 };
    let count = fragment.len();
    let w = crate::node_graph::ui::NODE_WIDTH;
    let at = graph.nodes.iter()
        .fold(None::<egui::Rect>, |r, n| {
            let b = egui::Rect::from_min_size(n.position, egui::vec2(w, crate::node_graph::ui::NODE_HEIGHT));
            Some(r.map_or(b, |r| r.union(b)))
        })
        .map(|r| egui::pos2(r.right() + 120.0, r.top()));
    let placed = paste(graph, subnets, fragment, at.or(Some(egui::pos2(40.0, 20.0))));
    let out_of = |g: &NodeGraphState| g.nodes.iter().find(|n| matches!(n.node_type, NodeType::Output))
        .map(|n| (n.id, n.inputs.first().and_then(|i| i.connected_output)));
    if let (Some((out, None)), Some((_, Some((src, socket))))) = (out_of(graph), out_of(other)) {
        if let Some(&new) = placed.get(&src.0) { graph.add_connection(new, socket, out, 0); }
    }
    if graph.view_flag.is_none() {
        graph.view_flag = other.view_flag.and_then(|v| placed.get(&v.0).copied());
    }
    graph.frame_request = true;
    count
}

/// Keep copied nodes for a later session: the system clipboard can lose
/// them when the program that copied them closes.
pub fn remember(text: &str) {
    if let Some(file) = crate::file_browser::config_path("clipboard.json") {
        if let Some(dir) = file.parent() { let _ = std::fs::create_dir_all(dir); }
        let _ = std::fs::write(file, text);
    }
}

/// The nodes copied last, from this session or an earlier one.
pub fn remembered() -> Option<Fragment> {
    let file = crate::file_browser::config_path("clipboard.json")?;
    Fragment::from_text(&std::fs::read_to_string(file).ok()?)
}

/// Remove everything except the Output node.
pub(crate) fn clear(graph: &mut NodeGraphState) {
    graph.connections.clear();
    graph.nodes.retain(|n| matches!(n.node_type, NodeType::Output));
    for n in &mut graph.nodes {
        for i in &mut n.inputs { i.connected_output = None; }
    }
    graph.selected_node = None;
    graph.selected_nodes.clear();
    graph.selected_connection = None;
    graph.view_flag = None;
    graph.dragging_node = None;
    graph.connecting_from = None;
}

/// Graph that splits a two-character mocap take for Unreal: per character,
/// one FBX with the animation and one skinned T-pose FBX.
pub fn mocap_split_template(graph: &mut NodeGraphState) {
    clear(graph);
    let p = |x: f32, y: f32| egui::pos2(x, y);

    let load = graph.add_node("Takes".into(),
        NodeType::LoadFbxDir { dir: String::new(), index: 0, take: 0 }, p(330.0, 20.0));
    let split = graph.add_node("Split".into(),
        NodeType::SplitSkeleton { picks: vec![SplitPick::Character(0), SplitPick::Character(1)] }, p(330.0, 120.0));
    graph.add_connection(load, 0, split, 0);

    let tpose_path = DEFAULT_WRITE_PATH.replace(".fbx", "_tpose.fbx");
    for c in 0..2usize {
        let x = 20.0 + c as f32 * 420.0;
        let n = c + 1;
        let anim = graph.add_node(format!("WriteAnim{n}"),
            NodeType::WriteFbx { path: DEFAULT_WRITE_PATH.into(), mesh: false }, p(x, 240.0));
        let tpose = graph.add_node(format!("TPose{n}"),
            NodeType::AutoTPose { set_hip_height: false, hip_height: 90.0 }, p(x + 200.0, 240.0));
        let fix = graph.add_node(format!("FixPose{n}"),
            NodeType::FixPose { edits: vec![] }, p(x + 200.0, 340.0));
        let skin = graph.add_node(format!("ProxySkin{n}"),
            NodeType::ProxySkin { thickness: 1.0 }, p(x + 200.0, 440.0));
        let write = graph.add_node(format!("WriteTPose{n}"),
            NodeType::WriteFbx { path: tpose_path.clone(), mesh: true }, p(x + 200.0, 540.0));
        graph.add_connection(split, c, anim, 0);
        graph.add_connection(split, c, tpose, 0);
        graph.add_connection(tpose, 0, fix, 0);
        graph.add_connection(fix, 0, skin, 0);
        graph.add_connection(skin, 0, write, 0);
    }

    if let Some(out) = graph.nodes.iter_mut().find(|n| matches!(n.node_type, NodeType::Output)) {
        out.position = p(330.0, 660.0);
    }
    graph.view_flag     = Some(split);
    graph.selected_node = Some(load);
    graph.selected_nodes = vec![load];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypass_passes_the_first_input_through() {
        let mut g = NodeGraphState::default();
        let cube = g.add_node("Cube".into(), NodeType::CreateCube { size: 2.0 }, egui::pos2(0.0, 0.0));
        let xf = g.add_node("Xf".into(), NodeType::Transform {
            translation: bevy::math::Vec3::new(5.0, 0.0, 0.0),
            rotation: bevy::math::Vec3::ZERO, scale: bevy::math::Vec3::ONE,
        }, egui::pos2(0.0, 0.0));
        g.add_connection(cube, 0, xf, 0);
        let pass = |_: crate::types::SubnetId, m: &crate::core::geo::Geo, _: Option<&crate::core::geo::Geo>| m.clone();
        let max_x = |g: &NodeGraphState| g.eval_node(xf, &mut Default::default(), &pass).unwrap().into_mesh()
            .vertices.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
        assert!((max_x(&g) - 6.0).abs() < 1e-5);
        let before = g.content_hash();
        g.toggle_bypass(xf);
        assert_ne!(before, g.content_hash());
        assert!((max_x(&g) - 1.0).abs() < 1e-5);
        // Output cannot be bypassed, and a bypassed node with nothing wired gives nothing.
        let out = g.nodes.iter().find(|n| matches!(n.node_type, NodeType::Output)).unwrap().id;
        g.toggle_bypass(out);
        assert!(!g.nodes.iter().find(|n| n.id == out).unwrap().bypassed);
        g.toggle_bypass(cube);
        assert!(g.eval_node(cube, &mut Default::default(), &pass).is_none());
    }

    #[test]
    fn graph_survives_save_and_load() {
        let mut g = NodeGraphState::default();
        mocap_split_template(&mut g);
        let cube = g.add_node("Cube".into(), NodeType::CreateCube { size: 2.0 }, egui::pos2(1.0, 2.0));
        let xf = g.add_node("Xf".into(), NodeType::Transform {
            translation: bevy::math::Vec3::new(1.0, 2.0, 3.0),
            rotation: bevy::math::Vec3::ZERO, scale: bevy::math::Vec3::ONE,
        }, egui::pos2(5.0, 6.0));
        g.add_connection(cube, 0, xf, 0);
        g.toggle_bypass(xf);
        // An Edit Poly node with an operation, a rule-based and a picked selection.
        use crate::core::poly::{ExtrudeMode, PolyOp, PolyOpKind, PolySelection, SelSource, SubLevel};
        let ep = g.add_node("EditPoly".into(), NodeType::EditPoly {
            ops: vec![
                PolyOp {
                    enabled: true,
                    selection: PolySelection { source: SelSource::ByNormal { dir: [0.0, 1.0, 0.0], angle: 5.0 }, grow: 1, ..Default::default() },
                    kind: PolyOpKind::Bevel { height: 0.5, outline: -0.1, mode: ExtrudeMode::LocalNormal },
                    collapsed: true,
                },
                PolyOp::new(
                    PolySelection { level: SubLevel::Border, ..Default::default() },
                    PolyOpKind::Transform { translate: [0.5, 0.0, 0.25], rotate: [0.0, 0.0, 0.0, 1.0], scale: [1.0, 2.0, 1.0], falloff: 0.0 }),
                PolyOp::new(Default::default(), PolyOpKind::MakePlanar { axis: None }),
            ],
            pending: PolySelection { level: SubLevel::Edge, edges: vec![[0, 1], [2, 3]], ..Default::default() },
            edit: Some(0),
            auto_collapse: true,
        }, egui::pos2(9.0, 9.0));
        g.add_connection(xf, 0, ep, 0);
        let json = to_json(&g);

        let mut h = NodeGraphState::default();
        from_json(&mut h, &json).unwrap();
        assert_eq!(h.nodes.len(), g.nodes.len());
        assert_eq!(h.connections.len(), g.connections.len());
        assert_eq!(h.view_flag, g.view_flag);
        assert!(h.nodes.iter().any(|n| n.bypassed));
        assert_eq!(to_json(&h), json);
        // Split keeps both outputs and their wires.
        let split = h.nodes.iter().find(|n| matches!(n.node_type, NodeType::SplitSkeleton { .. })).unwrap();
        assert_eq!(split.outputs.len(), 2);
        assert_eq!(h.connections.iter().filter(|c| c.from_node == split.id && c.from_output == 1).count(), 2);
        // New nodes do not collide with restored ids.
        let fresh = h.add_node("New".into(), NodeType::Merge, egui::pos2(0.0, 0.0));
        assert_eq!(h.nodes.iter().filter(|n| n.id == fresh).count(), 1);
    }

    /// What a Characterize node sets is saved with the graph and reaches
    /// the nodes after it.
    #[test]
    fn characterize_picks_are_saved_and_reach_the_clip() {
        use crate::core::human::{Human, Slot};
        let mut g = NodeGraphState::default();
        let take = g.add_node("Take".into(), NodeType::TestClip { seconds: 1.0, fps_num: 30, fps_den: 1 }, egui::pos2(0.0, 0.0));
        let who = g.add_node("Characterize".into(), NodeType::Characterize { picks: vec![(Slot::LeftHand, "Take01:LeftForeArm".into())] }, egui::pos2(0.0, 80.0));
        g.add_connection(take, 0, who, 0);
        let json = to_json(&g);
        let mut back = NodeGraphState::default();
        from_json(&mut back, &json).unwrap();
        let clip = back.eval_anim(who).unwrap();
        let h = Human::of(&clip);
        assert_eq!(h.get(Slot::LeftHand).map(|j| clip.joints[j].name.as_str()), Some("Take01:LeftForeArm"));
        assert_eq!(h.get(Slot::RightHand).map(|j| clip.joints[j].name.as_str()), Some("Take01:RightHand"));
        assert_eq!(h.picked, vec![Slot::LeftHand]);
    }

    /// Copied nodes come back through text (the clipboard) with their
    /// parameters, the wires between them and their ICE tree, as new nodes.
    #[test]
    fn copied_nodes_paste_with_their_wires_and_trees() {
        use crate::types::SubnetNodeType;
        let mut subnets = SubnetStore::default();
        let mut g = NodeGraphState::default();
        let cube = g.add_node("Cube".into(), NodeType::CreateCube { size: 2.0 }, egui::pos2(100.0, 0.0));
        let sid = subnets.create_subnet("Spherify".into());
        {
            let sg = subnets.get_mut(sid).unwrap();
            let (input, output) = (sg.nodes[0].id, sg.nodes[1].id);
            let norm = sg.add_node("Normalize".into(), SubnetNodeType::Normalize, egui::pos2(200.0, 200.0));
            sg.add_connection(input, 1, norm, 0);
            sg.add_connection(norm, 0, output, 0);
        }
        let ice = g.add_node("ICE".into(), NodeType::Subnet { id: sid, name: "Spherify".into() }, egui::pos2(100.0, 100.0));
        g.add_connection(cube, 0, ice, 0);
        g.toggle_bypass(cube);
        let out = g.nodes.iter().find(|n| matches!(n.node_type, NodeType::Output)).unwrap().id;
        g.add_connection(ice, 0, out, 0);

        // Output is not copied, nor the wire to it.
        let text = copy(&g, &subnets, &[cube, ice, out]).unwrap().to_text();
        let fragment = Fragment::from_text(&text).unwrap();
        assert_eq!(fragment.len(), 2);

        let placed = paste(&mut g, &mut subnets, fragment, Some(egui::pos2(500.0, 300.0)));
        let (new_cube, new_ice) = (placed[&cube.0], placed[&ice.0]);
        assert_eq!(g.nodes.len(), 5);
        assert_eq!(g.selected_nodes.len(), 2);
        let node = |id: NodeId| g.nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!(node(new_cube).position, egui::pos2(500.0, 300.0));
        assert_eq!(node(new_ice).position, egui::pos2(500.0, 400.0));
        assert!(node(new_cube).bypassed);
        assert_eq!(node(new_ice).inputs[0].connected_output, Some((new_cube, 0)));
        assert_eq!(g.connections.iter().filter(|c| c.to_node == out).count(), 1, "the pasted ICE node is not wired to Output");
        // A tree of its own, with the same nodes and wires.
        let NodeType::Subnet { id: new_sid, .. } = node(new_ice).node_type else { panic!() };
        assert_ne!(new_sid, sid);
        let (a, b) = (subnets.get(sid).unwrap(), subnets.get(new_sid).unwrap());
        assert_eq!(a.nodes.len(), b.nodes.len());
        assert_eq!(a.connections.len(), b.connections.len());
        let sub_out = b.nodes.iter().find(|n| matches!(n.node_type, SubnetNodeType::SubOutput)).unwrap();
        assert!(sub_out.inputs[0].connected_output.is_some());
        subnets.get_mut(sid).unwrap().nodes.retain(|n| !matches!(n.node_type, SubnetNodeType::Normalize));
        assert_eq!(subnets.get(new_sid).unwrap().nodes.len(), 3, "editing one tree leaves the other");
    }

    #[test]
    fn other_text_is_not_nodes() {
        assert!(Fragment::from_text("hello").is_none());
        assert!(Fragment::from_text(r#"{"version":1,"nodes":[],"connections":[],"view_flag":null}"#).is_none());
        let g = NodeGraphState::default();
        let out = g.nodes[0].id;
        assert!(copy(&g, &SubnetStore::default(), &[out]).is_none(), "Output alone copies nothing");
    }

    /// A graph added to another lands to the right of its nodes, and takes
    /// Output only when Output was free.
    #[test]
    fn a_graph_added_sits_beside_and_wires_a_free_output() {
        let mut subnets = SubnetStore::default();
        let mut other = NodeGraphState::default();
        mocap_split_template(&mut other);
        let cube = other.add_node("Cube".into(), NodeType::CreateCube { size: 1.0 }, egui::pos2(0.0, 0.0));
        let other_out = other.nodes.iter().find(|n| matches!(n.node_type, NodeType::Output)).unwrap().id;
        other.add_connection(cube, 0, other_out, 0);
        let added = other.nodes.len() - 1;

        let mut g = NodeGraphState::default();
        let mine = g.add_node("Sphere".into(), NodeType::CreateSphere { radius: 1.0, segments: 8 }, egui::pos2(0.0, 0.0));
        assert_eq!(add_graph(&mut g, &mut subnets, &other), added);
        assert_eq!(g.nodes.len(), 2 + added);
        assert_eq!(g.nodes.iter().filter(|n| matches!(n.node_type, NodeType::Output)).count(), 1);
        let right = g.nodes.iter().find(|n| n.id == mine).unwrap().position.x + crate::node_graph::ui::NODE_WIDTH;
        assert!(g.nodes.iter().filter(|n| g.selected_nodes.contains(&n.id)).all(|n| n.position.x > right));
        let out = g.nodes.iter().find(|n| matches!(n.node_type, NodeType::Output)).unwrap();
        let (src, _) = out.inputs[0].connected_output.expect("Output was free: it takes the added cube");
        assert!(matches!(g.nodes.iter().find(|n| n.id == src).unwrap().node_type, NodeType::CreateCube { .. }));
        assert!(g.view_flag.is_some(), "nothing was viewed: the added graph's view comes with it");

        // Output already wired stays as it was.
        let mut h = NodeGraphState::default();
        let s = h.add_node("Sphere".into(), NodeType::CreateSphere { radius: 1.0, segments: 8 }, egui::pos2(0.0, 0.0));
        let h_out = h.nodes.iter().find(|n| matches!(n.node_type, NodeType::Output)).unwrap().id;
        h.add_connection(s, 0, h_out, 0);
        add_graph(&mut h, &mut subnets, &other);
        assert_eq!(h.nodes.iter().find(|n| n.id == h_out).unwrap().inputs[0].connected_output, Some((s, 0)));
    }

    #[test]
    fn bad_json_leaves_graph_alone() {
        let mut g = NodeGraphState::default();
        mocap_split_template(&mut g);
        let before = to_json(&g);
        assert!(from_json(&mut g, "{ not json").is_err());
        assert!(from_json(&mut g, r#"{"version":1,"nodes":[],"connections":[],"view_flag":null}"#).is_err());
        assert_eq!(to_json(&g), before);
    }

    /// A graph saved before Body Collide had its display and Transform took
    /// clips: Calamari folds into Body Collide, Transform Clip becomes
    /// Transform with the same rotation.
    #[test]
    fn older_graphs_are_brought_up_to_date() {
        let json = r#"{"version":1,"nodes":[
            {"id":0,"name":"Output","position":[0,500],"node_type":"Output"},
            {"id":2,"name":"Take","position":[0,0],"node_type":{"TestClip":{"seconds":1.0,"fps_num":30,"fps_den":1}}},
            {"id":3,"name":"Turn","position":[0,50],"node_type":{"TransformClip":{"translate":[1.0,0.0,0.0],"rotate":[0.0,90.0,0.0],"scale":2.0}}},
            {"id":4,"name":"Ragdoll","position":[0,100],"node_type":{"Ragdoll":{"settings":{"margin":1.2,"friction":0.5,"self_collision":true,"self_slack":3.0,"stiffness":1.0,"release":60.0,"ghost_depth":12.0,"sink":0.0,"fade_out":6,"fade_in":10,"limb_limit":45.0,"smooth":2,"detail":1}}}},
            {"id":5,"name":"Hulls","position":[0,200],"node_type":{"Calamari":{"hulls":true,"detail":1}}}
        ],"connections":[[2,0,3,0],[3,0,4,0],[4,0,5,0],[5,0,0,0]],"view_flag":5}"#;
        let mut g = NodeGraphState::default();
        from_json(&mut g, json).unwrap();
        assert!(!g.nodes.iter().any(|n| matches!(n.node_type, NodeType::Calamari { .. } | NodeType::TransformClip { .. })));
        let rag = g.nodes.iter().find(|n| n.id == NodeId(4)).unwrap();
        assert!(matches!(rag.node_type, NodeType::Ragdoll { view: crate::types::BodyView::Hulls, .. }));
        assert_eq!(g.view_flag, Some(NodeId(4)));
        let out = g.nodes.iter().find(|n| matches!(n.node_type, NodeType::Output)).unwrap();
        assert_eq!(out.inputs[0].connected_output, Some((NodeId(4), 0)), "Output is wired to Body Collide now");
        // Same motion as the old Transform Clip.
        let old = crate::core::anim::create_test_clip(1.0, crate::core::anim::FrameRate::new(30, 1))
            .transformed(bevy::math::Vec3::new(1.0, 0.0, 0.0), bevy::math::Vec3::new(0.0, 90.0, 0.0), 2.0);
        let new = g.eval_anim(NodeId(3)).unwrap();
        for f in [0, 11] {
            let (a, b) = (old.world_pose(f), new.world_pose(f));
            for j in 0..a.len() { assert!((a[j].w_axis - b[j].w_axis).length() < 1e-4); }
        }
    }
}
