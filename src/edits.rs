//! What a scene network has changed in a composed stage.
//!
//! Every packed primitive from a stage carries its `source`: itself as it
//! was loaded. Edits are what no longer shares with that source. Because
//! columns are shared and copied only when written, this is a comparison of
//! pointers, not of values: an attribute still pointing at the loaded column
//! is unedited, however large it is.
//!
//! An `EditSet` is the list of those differences for the primitives arriving
//! somewhere (the output, or any node). Writing USD turns it into opinions
//! (an override layer); a render delegate uses it to update only what
//! changed. Comparing two nodes' outputs, rather than one against the
//! source, gives the edits of the nodes between them: Solaris-style stacked
//! layers come from the same function.

// Read by Write USD and the delegates, next.
#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::Arc;

use crate::core::geo::{Context, Geo};
use crate::types::{NamedMesh, Placement, Purpose};
use crate::usd_scene::StageTree;

/// What changed in one primitive.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrimEdit {
    pub path: String,
    /// The new placement, when it moved (world transform, root correction
    /// included: see `StageTree::root`).
    pub place: Option<Placement>,
    /// The polygons or curves changed: point counts and indices.
    pub topology: bool,
    /// Attributes written or added, by context and name (`points` among them
    /// when the points moved).
    pub attrs: Vec<(Context, Arc<str>)>,
    /// Attributes the source had and the primitive no longer has.
    pub removed: Vec<(Context, Arc<str>)>,
    /// The new material binding, when it changed (`None` inside: unbound).
    pub material: Option<Option<String>>,
    /// The new purpose, when it changed.
    pub purpose: Option<Purpose>,
    /// The geometry can no longer be traced to the source (several
    /// primitives merged into one): everything is to be written, from the
    /// primitive's mesh, where it is drawn.
    pub replaced: bool,
}

impl PrimEdit {
    pub fn is_empty(&self) -> bool {
        self.place.is_none() && !self.topology && self.attrs.is_empty() && self.removed.is_empty()
            && self.material.is_none() && self.purpose.is_none() && !self.replaced
    }
}

/// Everything changed in one stage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditSet {
    /// Primitives of the stage that changed, in the order they arrive.
    pub prims: Vec<PrimEdit>,
    /// Geometry prims of the stage that no longer arrive (pruned, or merged
    /// into another): to be deactivated.
    pub removed: Vec<String>,
    /// Primitives that did not come from the stage (made by nodes): to be
    /// added.
    pub added: Vec<String>,
}

impl EditSet {
    pub fn is_empty(&self) -> bool {
        self.prims.is_empty() && self.removed.is_empty() && self.added.is_empty()
    }
}

/// What one primitive changed against its source. `None` without a source.
pub fn prim_edit(p: &NamedMesh) -> Option<PrimEdit> {
    let src = p.source.as_ref()?;
    let mut e = PrimEdit { path: p.path.clone(), ..Default::default() };
    if !p.place.same(&src.place) { e.place = Some(p.place.clone()); }
    if p.material != src.material { e.material = Some(p.material.clone()); }
    if p.purpose != src.purpose { e.purpose = Some(p.purpose); }
    match &p.geo {
        None => e.replaced = true,
        Some(geo) if Arc::ptr_eq(geo, &src.geo) => {}
        Some(geo) => diff_geo(&src.geo, geo, &mut e),
    }
    Some(e)
}

fn diff_geo(src: &Geo, cur: &Geo, e: &mut PrimEdit) {
    e.topology = !cur.topology.same(&src.topology);
    for ctx in Context::ALL {
        for (name, attr) in cur.attrs(ctx) {
            if !src.attr(ctx, name).is_some_and(|s| s.same(attr)) {
                e.attrs.push((ctx, Arc::from(name)));
            }
        }
        for (name, _) in src.attrs(ctx) {
            if cur.attr(ctx, name).is_none() {
                e.removed.push((ctx, Arc::from(name)));
            }
        }
    }
}

/// The edits of the primitives of one stage.
pub fn edit_set(prims: &[NamedMesh], tree: &StageTree) -> EditSet {
    let mut set = EditSet::default();
    let mut present: HashSet<&str> = HashSet::new();
    for p in prims.iter().filter(|p| p.stage.as_deref().is_some_and(|t| std::ptr::eq(t, tree))) {
        present.insert(p.path.as_str());
        match prim_edit(p) {
            Some(e) if !e.is_empty() => set.prims.push(e),
            Some(_) => {}
            None => set.added.push(p.path.clone()),
        }
    }
    // Geometry the stage has (and that was drawn: hidden prims never arrive)
    // which no longer arrives.
    let geometry = ["Mesh", "BasisCurves", "NurbsCurves", "HermiteCurves", "Points"];
    set.removed = tree.nodes.iter()
        .filter(|n| !n.hidden && geometry.contains(&n.type_name.as_str()) && !present.contains(n.path.as_str()))
        .map(|n| n.path.clone())
        .collect();
    set
}

/// The edits made between two points of a network: what `after` changed
/// relative to `before`, primitive by primitive, by path. The layer of the
/// nodes in between, for stacking edits as Solaris does.
pub fn edits_between(before: &[NamedMesh], after: &[NamedMesh]) -> EditSet {
    let mut set = EditSet::default();
    let earlier: std::collections::HashMap<&str, &NamedMesh> = before.iter().map(|p| (p.path.as_str(), p)).collect();
    for p in after {
        let Some(b) = earlier.get(p.path.as_str()) else { set.added.push(p.path.clone()); continue };
        // Measure against the earlier primitive as if it were the source.
        let base = NamedMesh {
            source: b.geo.clone().map(|geo| Arc::new(crate::types::Source {
                geo, place: b.place.clone(), material: b.material.clone(), purpose: b.purpose,
            })),
            ..p.clone()
        };
        match prim_edit(&base) {
            Some(e) if !e.is_empty() => set.prims.push(e),
            Some(_) => {}
            None => set.prims.push(PrimEdit { path: p.path.clone(), replaced: true, ..Default::default() }),
        }
    }
    let now: HashSet<&str> = after.iter().map(|p| p.path.as_str()).collect();
    set.removed = before.iter().filter(|p| !now.contains(p.path.as_str())).map(|p| p.path.clone()).collect();
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geo::{Attr, Column, Role, POINTS, ST};
    use crate::types::{MeshData, Source};
    use crate::usd_scene::StageNode;
    use bevy::math::{Mat4, Vec3};

    fn node(path: &str, ty: &str) -> StageNode {
        StageNode { path: path.into(), name: path.rsplit('/').next().unwrap().into(), type_name: ty.into(), depth: 1, ..Default::default() }
    }

    /// A loaded quad: its geometry, placement and source, on a stage.
    fn loaded(path: &str, tree: &Arc<StageTree>) -> NamedMesh {
        let mut geo = Geo::from_polygons(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]], vec![4], vec![0, 1, 2, 3]);
        geo.set(Context::Corner, ST, Attr::new(Column::Vec2(Arc::new(vec![[0.0, 0.0]; 4])), Role::TexCoord));
        let geo = Arc::new(geo);
        let place = Placement::One(Mat4::IDENTITY);
        NamedMesh {
            mesh: Arc::new(geo.to_mesh()), geo: Some(geo.clone()), place: place.clone(), stage: Some(tree.clone()),
            material: Some("/looks/a".into()),
            source: Some(Arc::new(Source { geo, place, material: Some("/looks/a".into()), purpose: Purpose::Default })),
            ..NamedMesh::new(path.into(), MeshData::default())
        }
    }

    fn stage() -> Arc<StageTree> {
        Arc::new(StageTree { nodes: vec![node("/a", "Mesh"), node("/b", "Mesh"), node("/c", "Mesh"), node("/looks", "Scope")], ..Default::default() })
    }

    #[test]
    fn untouched_primitives_have_no_edits() {
        let tree = stage();
        let prims = vec![loaded("/a", &tree), loaded("/b", &tree), loaded("/c", &tree)];
        assert!(edit_set(&prims, &tree).is_empty());
    }

    #[test]
    fn each_kind_of_change_is_found_and_nothing_else() {
        let tree = stage();
        let (mut a, mut b, c) = (loaded("/a", &tree), loaded("/b", &tree), loaded("/c", &tree));
        // a: points moved, so only `points` is new.
        let mut g = (**a.geo.as_ref().unwrap()).clone();
        g.points_mut()[0] = [0.0, 0.0, 1.0];
        a.geo = Some(Arc::new(g));
        // b: moved, rebound, and `st` removed.
        b.place = b.place.moved(Mat4::from_translation(Vec3::X));
        b.material = Some("/looks/b".into());
        let mut g = (**b.geo.as_ref().unwrap()).clone();
        g.remove(Context::Corner, ST);
        b.geo = Some(Arc::new(g));
        // c: pruned. And a primitive made by a node.
        let made = NamedMesh::new("/made".into(), MeshData::default());

        let set = edit_set(&[a, b, made], &tree);
        let ea = &set.prims[0];
        assert_eq!((ea.path.as_str(), ea.attrs.clone(), ea.topology, ea.place.is_some()), ("/a", vec![(Context::Point, Arc::from(POINTS))], false, false));
        let eb = &set.prims[1];
        assert!(eb.place.is_some() && eb.attrs.is_empty());
        assert_eq!(eb.material, Some(Some("/looks/b".into())));
        assert_eq!(eb.removed, vec![(Context::Corner, Arc::from(ST))]);
        assert_eq!(set.removed, vec!["/c".to_string()]);
        // Not from this stage: listed by the stage it belongs to, not here.
        assert!(set.added.is_empty());
    }

    #[test]
    fn merged_geometry_is_replaced() {
        let tree = stage();
        let mut a = loaded("/a", &tree);
        a.geo = None;
        let set = edit_set(&[a], &tree);
        assert!(set.prims[0].replaced);
        assert_eq!(set.removed, vec!["/b".to_string(), "/c".to_string()]);
    }

    #[test]
    fn edits_between_two_nodes_are_the_layer_in_between() {
        let tree = stage();
        let before = vec![loaded("/a", &tree), loaded("/b", &tree)];
        let mut after = before.clone();
        after[1].place = after[1].place.moved(Mat4::from_translation(Vec3::Y));
        after.remove(0);
        let layer = edits_between(&before, &after);
        assert_eq!(layer.prims.len(), 1);
        assert!(layer.prims[0].place.is_some() && layer.prims[0].attrs.is_empty());
        assert_eq!(layer.removed, vec!["/a".to_string()]);
    }
}
