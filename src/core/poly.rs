//! Polygon modelling: the mesh, sub-object selections, the operations of the
//! Edit Poly node, and viewport picking.
//!
//! Everything here is plain data and maths with no Bevy systems, so it can be
//! tested without a window.

use std::collections::{HashMap, HashSet};

use bevy::math::{Mat4, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::types::MeshData;

// ============================================================================
// MESH
// ============================================================================

/// Polygons of any size over shared vertices. Winding is counter-clockwise
/// seen from outside.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PolyMesh {
    pub verts: Vec<Vec3>,
    pub polys: Vec<Vec<u32>>,
}

/// Edge as a sorted vertex pair.
pub fn edge_key(a: u32, b: u32) -> [u32; 2] { if a < b { [a, b] } else { [b, a] } }

impl PolyMesh {
    pub fn from_mesh(m: &MeshData) -> Self {
        Self {
            verts: m.vertices.iter().map(|v| Vec3::from_array(*v)).collect(),
            polys: m.polygons(),
        }
    }

    pub fn to_mesh(&self) -> MeshData {
        MeshData::from_polys(self.verts.iter().map(|v| v.to_array()).collect(), self.polys.clone())
    }

    /// Unit normal (Newell's method, so non-planar polygons are fine).
    pub fn normal(&self, p: usize) -> Vec3 { self.area_normal(p).normalize_or_zero() }

    /// Normal scaled by twice the polygon's area.
    pub(crate) fn area_normal(&self, p: usize) -> Vec3 {
        let poly = &self.polys[p];
        let mut n = Vec3::ZERO;
        for i in 0..poly.len() {
            let a = self.verts[poly[i] as usize];
            let b = self.verts[poly[(i + 1) % poly.len()] as usize];
            n += a.cross(b);
        }
        n
    }

    pub fn centroid(&self, p: usize) -> Vec3 {
        let poly = &self.polys[p];
        poly.iter().map(|v| self.verts[*v as usize]).sum::<Vec3>() / poly.len().max(1) as f32
    }

    /// Unique edges, in the order they first appear.
    pub fn edges(&self) -> Vec<[u32; 2]> {
        let mut seen = HashSet::new();
        let mut out = vec![];
        for poly in &self.polys {
            for i in 0..poly.len() {
                let e = edge_key(poly[i], poly[(i + 1) % poly.len()]);
                if e[0] != e[1] && seen.insert(e) { out.push(e); }
            }
        }
        out
    }

    /// Polygons around each vertex.
    fn vertex_polys(&self) -> Vec<Vec<u32>> {
        let mut out = vec![vec![]; self.verts.len()];
        for (p, poly) in self.polys.iter().enumerate() {
            for v in poly { out[*v as usize].push(p as u32); }
        }
        out
    }

    /// Volume enclosed by a closed mesh (signed: positive when normals point out).
    pub fn volume(&self) -> f32 {
        let mut v = 0.0;
        for poly in &self.polys {
            for i in 1..poly.len().saturating_sub(1) {
                let (a, b, c) = (self.verts[poly[0] as usize], self.verts[poly[i] as usize], self.verts[poly[i + 1] as usize]);
                v += a.dot(b.cross(c)) / 6.0;
            }
        }
        v
    }

    /// True when every edge is used by exactly two polygons, in opposite
    /// directions: a closed surface with consistent normals.
    pub fn is_closed(&self) -> bool {
        let mut dir: HashMap<(u32, u32), u32> = HashMap::new();
        for poly in &self.polys {
            for i in 0..poly.len() {
                *dir.entry((poly[i], poly[(i + 1) % poly.len()])).or_default() += 1;
            }
        }
        dir.iter().all(|((a, b), n)| *n == 1 && dir.get(&(*b, *a)) == Some(&1))
    }

    /// Drop vertices no polygon uses and renumber the rest.
    pub(crate) fn compact(&mut self) {
        let mut used = vec![false; self.verts.len()];
        for poly in &self.polys { for v in poly { used[*v as usize] = true; } }
        if used.iter().all(|u| *u) { return; }
        let mut remap = vec![u32::MAX; self.verts.len()];
        let mut verts = Vec::with_capacity(self.verts.len());
        for (i, v) in self.verts.iter().enumerate() {
            if used[i] { remap[i] = verts.len() as u32; verts.push(*v); }
        }
        for poly in &mut self.polys { for v in poly.iter_mut() { *v = remap[*v as usize]; } }
        self.verts = verts;
    }
}

// ============================================================================
// SELECTION
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubLevel {
    Vertex,
    Edge,
    /// Open edges, picked a whole border at a time. Stored as edges.
    Border,
    Polygon,
    /// Polygons, picked a connected piece at a time. Stored as polygons.
    Element,
}

impl SubLevel {
    /// The kind of component the level stores.
    pub fn base(self) -> SubLevel {
        match self {
            SubLevel::Border  => SubLevel::Edge,
            SubLevel::Element => SubLevel::Polygon,
            other => other,
        }
    }
}

/// Where a selection comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SelSource {
    /// Components clicked in the viewport, stored by index.
    Picked,
    All,
    /// Polygons facing within `angle` degrees of `dir`.
    ByNormal { dir: [f32; 3], angle: f32 },
    /// Components inside an axis-aligned box.
    InBox { min: [f32; 3], max: [f32; 3] },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolySelection {
    pub level:  SubLevel,
    pub source: SelSource,
    /// Picked components. Each level keeps its own list, as in Edit Poly.
    pub verts:  Vec<u32>,
    pub edges:  Vec<[u32; 2]>,
    pub polys:  Vec<u32>,
    /// Steps to grow (positive) or shrink (negative) the selection.
    pub grow:   i32,
    pub invert: bool,
}

impl Default for PolySelection {
    fn default() -> Self {
        Self {
            level: SubLevel::Polygon, source: SelSource::Picked,
            verts: vec![], edges: vec![], polys: vec![], grow: 0, invert: false,
        }
    }
}

/// A selection worked out against one mesh.
#[derive(Clone, Debug, Default)]
pub struct Resolved {
    pub verts: Vec<bool>,
    pub edges: HashSet<[u32; 2]>,
    pub polys: Vec<bool>,
}

/// One thing under the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component { Vertex(u32), Edge([u32; 2]), Polygon(u32) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickMode { Replace, Add, Remove }

impl PolySelection {
    pub fn picked_polys(polys: Vec<u32>) -> Self {
        Self { polys, ..Default::default() }
    }

    /// The selected components of the current level. The other two sets of
    /// `Resolved` are left empty.
    pub fn resolve(&self, mesh: &PolyMesh) -> Resolved {
        let nv = mesh.verts.len();
        let np = mesh.polys.len();
        let inside = |p: Vec3, min: &[f32; 3], max: &[f32; 3]| {
            (0..3).all(|a| p[a] >= min[a].min(max[a]) - 1e-5 && p[a] <= max[a].max(min[a]) + 1e-5)
        };
        // Polygons matched by the rule, for the rules defined on polygons.
        let facing: Option<Vec<bool>> = match &self.source {
            SelSource::ByNormal { dir, angle } => {
                let d = Vec3::from_array(*dir).normalize_or_zero();
                let limit = angle.to_radians().cos() - 1e-5;
                Some((0..np).map(|p| mesh.normal(p).dot(d) >= limit).collect())
            }
            _ => None,
        };

        let mut out = Resolved { verts: vec![false; nv], edges: HashSet::new(), polys: vec![false; np] };
        match self.level.base() {
            SubLevel::Polygon => {
                match &self.source {
                    SelSource::Picked => for p in &self.polys { if (*p as usize) < np { out.polys[*p as usize] = true; } },
                    SelSource::All    => out.polys.iter_mut().for_each(|s| *s = true),
                    SelSource::ByNormal { .. } => out.polys = facing.clone().unwrap(),
                    SelSource::InBox { min, max } => for p in 0..np { out.polys[p] = inside(mesh.centroid(p), min, max); },
                }
                if self.level == SubLevel::Element {
                    let element = mesh.elements();
                    let hit: HashSet<usize> = (0..np).filter(|p| out.polys[*p]).map(|p| element[p]).collect();
                    for p in 0..np { out.polys[p] = hit.contains(&element[p]); }
                }
                let around = mesh.vertex_polys();
                for _ in 0..self.grow.max(0) {
                    let mut touched = vec![false; nv];
                    for (p, poly) in mesh.polys.iter().enumerate() {
                        if out.polys[p] { for v in poly { touched[*v as usize] = true; } }
                    }
                    for (p, poly) in mesh.polys.iter().enumerate() {
                        if poly.iter().any(|v| touched[*v as usize]) { out.polys[p] = true; }
                    }
                }
                for _ in 0..(-self.grow).max(0) {
                    let prev = out.polys.clone();
                    for (p, poly) in mesh.polys.iter().enumerate() {
                        if prev[p] && poly.iter().any(|v| around[*v as usize].iter().any(|q| !prev[*q as usize])) {
                            out.polys[p] = false;
                        }
                    }
                }
                if self.invert { out.polys.iter_mut().for_each(|s| *s = !*s); }
            }

            SubLevel::Vertex => {
                match &self.source {
                    SelSource::Picked => for v in &self.verts { if (*v as usize) < nv { out.verts[*v as usize] = true; } },
                    SelSource::All    => out.verts.iter_mut().for_each(|s| *s = true),
                    SelSource::ByNormal { .. } => {
                        let f = facing.as_ref().unwrap();
                        for (p, poly) in mesh.polys.iter().enumerate() {
                            if f[p] { for v in poly { out.verts[*v as usize] = true; } }
                        }
                    }
                    SelSource::InBox { min, max } => for v in 0..nv { out.verts[v] = inside(mesh.verts[v], min, max); },
                }
                let edges = mesh.edges();
                for _ in 0..self.grow.max(0) {
                    let prev = out.verts.clone();
                    for e in &edges {
                        if prev[e[0] as usize] || prev[e[1] as usize] {
                            out.verts[e[0] as usize] = true;
                            out.verts[e[1] as usize] = true;
                        }
                    }
                }
                for _ in 0..(-self.grow).max(0) {
                    let prev = out.verts.clone();
                    for e in &edges {
                        if !prev[e[0] as usize] { out.verts[e[1] as usize] = false; }
                        if !prev[e[1] as usize] { out.verts[e[0] as usize] = false; }
                    }
                }
                if self.invert { out.verts.iter_mut().for_each(|s| *s = !*s); }
            }

            _ => {
                let mut edges = mesh.edges();
                if self.level == SubLevel::Border {
                    let open = mesh.open_edges();
                    edges.retain(|e| open.contains(e));
                }
                let known: HashSet<[u32; 2]> = edges.iter().copied().collect();
                match &self.source {
                    SelSource::Picked => for e in &self.edges {
                        let k = edge_key(e[0], e[1]);
                        if known.contains(&k) { out.edges.insert(k); }
                    },
                    SelSource::All => out.edges = known.clone(),
                    SelSource::ByNormal { .. } => {
                        let f = facing.as_ref().unwrap();
                        for (p, poly) in mesh.polys.iter().enumerate() {
                            if !f[p] { continue; }
                            for i in 0..poly.len() {
                                let k = edge_key(poly[i], poly[(i + 1) % poly.len()]);
                                if known.contains(&k) { out.edges.insert(k); }
                            }
                        }
                    }
                    SelSource::InBox { min, max } => for e in &edges {
                        if inside(mesh.verts[e[0] as usize], min, max) && inside(mesh.verts[e[1] as usize], min, max) {
                            out.edges.insert(*e);
                        }
                    },
                }
                for _ in 0..self.grow.max(0) {
                    let mut touched = vec![false; nv];
                    for e in &out.edges { touched[e[0] as usize] = true; touched[e[1] as usize] = true; }
                    for e in &edges {
                        if touched[e[0] as usize] || touched[e[1] as usize] { out.edges.insert(*e); }
                    }
                }
                for _ in 0..(-self.grow).max(0) {
                    let mut open = vec![false; nv];   // vertex touches an unselected edge
                    for e in &edges {
                        if !out.edges.contains(e) { open[e[0] as usize] = true; open[e[1] as usize] = true; }
                    }
                    out.edges.retain(|e| !open[e[0] as usize] && !open[e[1] as usize]);
                }
                if self.invert { out.edges = known.difference(&out.edges).copied().collect(); }
            }
        }
        out
    }

    /// Polygons the operations act on. A vertex or edge selection covers the
    /// polygons it fully surrounds.
    pub fn poly_mask(&self, mesh: &PolyMesh) -> Vec<bool> {
        let r = self.resolve(mesh);
        match self.level.base() {
            SubLevel::Polygon => r.polys,
            SubLevel::Vertex  => mesh.polys.iter()
                .map(|poly| !poly.is_empty() && poly.iter().all(|v| r.verts[*v as usize]))
                .collect(),
            _ => mesh.polys.iter()
                .map(|poly| !poly.is_empty() && (0..poly.len())
                    .all(|i| r.edges.contains(&edge_key(poly[i], poly[(i + 1) % poly.len()]))))
                .collect(),
        }
    }

    /// Vertices the selection touches, whatever its level.
    pub fn vertex_set(&self, mesh: &PolyMesh) -> Vec<bool> {
        let r = self.resolve(mesh);
        match self.level.base() {
            SubLevel::Vertex => r.verts,
            SubLevel::Polygon => {
                let mut out = vec![false; mesh.verts.len()];
                for (p, poly) in mesh.polys.iter().enumerate() {
                    if r.polys[p] { for v in poly { out[*v as usize] = true; } }
                }
                out
            }
            _ => {
                let mut out = vec![false; mesh.verts.len()];
                for e in &r.edges { out[e[0] as usize] = true; out[e[1] as usize] = true; }
                out
            }
        }
    }

    /// Edges the selection covers: the selected edges, the edges between
    /// selected vertices, or the edges of selected polygons.
    pub fn edge_set(&self, mesh: &PolyMesh) -> HashSet<[u32; 2]> {
        let r = self.resolve(mesh);
        match self.level.base() {
            SubLevel::Vertex => mesh.edges().into_iter()
                .filter(|e| r.verts[e[0] as usize] && r.verts[e[1] as usize]).collect(),
            SubLevel::Polygon => {
                let mut out = HashSet::new();
                for (p, poly) in mesh.polys.iter().enumerate() {
                    if !r.polys[p] { continue; }
                    for i in 0..poly.len() { out.insert(edge_key(poly[i], poly[(i + 1) % poly.len()])); }
                }
                out
            }
            _ => r.edges,
        }
    }

    /// Centre of the selected vertices.
    pub fn centre(&self, mesh: &PolyMesh) -> Option<Vec3> {
        let set = self.vertex_set(mesh);
        let picked: Vec<Vec3> = (0..mesh.verts.len()).filter(|v| set[*v]).map(|v| mesh.verts[v]).collect();
        (!picked.is_empty()).then(|| picked.iter().sum::<Vec3>() / picked.len() as f32)
    }

    /// Grow an edge selection into loops or rings.
    pub fn expand_edges(&mut self, mesh: &PolyMesh, ring: bool) {
        if self.level.base() != SubLevel::Edge { return; }
        let start = self.resolve(mesh).edges;
        let grown = if ring { mesh.edge_ring(&start) } else { mesh.edge_loop(&start) };
        self.edges = grown.into_iter().collect();
        self.edges.sort();
        self.source = SelSource::Picked;
        self.grow   = 0;
        self.invert = false;
    }

    /// Number of selected components at the current level.
    pub fn count(&self, mesh: &PolyMesh) -> usize {
        let r = self.resolve(mesh);
        match self.level.base() {
            SubLevel::Vertex  => r.verts.iter().filter(|s| **s).count(),
            SubLevel::Polygon => r.polys.iter().filter(|s| **s).count(),
            _                 => r.edges.len(),
        }
    }

    /// Turn whatever is selected now into an explicit picked list, so clicks
    /// can add to or remove from a rule-based selection.
    pub fn bake(&mut self, mesh: &PolyMesh) {
        if self.source == SelSource::Picked && self.grow == 0 && !self.invert { return; }
        let r = self.resolve(mesh);
        match self.level.base() {
            SubLevel::Vertex  => self.verts = (0..r.verts.len() as u32).filter(|v| r.verts[*v as usize]).collect(),
            SubLevel::Polygon => self.polys = (0..r.polys.len() as u32).filter(|p| r.polys[*p as usize]).collect(),
            _                 => { self.edges = r.edges.into_iter().collect(); self.edges.sort(); }
        }
        self.source = SelSource::Picked;
        self.grow   = 0;
        self.invert = false;
    }

    /// Apply a click or a box selection.
    pub fn apply_pick(&mut self, mesh: &PolyMesh, picked: &[Component], mode: PickMode) {
        self.bake(mesh);
        fn update<T: PartialEq + Copy>(list: &mut Vec<T>, items: Vec<T>, mode: PickMode) {
            match mode {
                PickMode::Replace => *list = items,
                PickMode::Add     => for i in items { if !list.contains(&i) { list.push(i); } },
                PickMode::Remove  => list.retain(|x| !items.contains(x)),
            }
        }
        match self.level.base() {
            SubLevel::Vertex => update(&mut self.verts,
                picked.iter().filter_map(|c| if let Component::Vertex(v) = c { Some(*v) } else { None }).collect(), mode),
            SubLevel::Edge | SubLevel::Border => update(&mut self.edges,
                picked.iter().filter_map(|c| if let Component::Edge(e) = c { Some(edge_key(e[0], e[1])) } else { None }).collect(), mode),
            SubLevel::Polygon | SubLevel::Element => update(&mut self.polys,
                picked.iter().filter_map(|c| if let Component::Polygon(p) = c { Some(*p) } else { None }).collect(), mode),
        }
    }
}

// ============================================================================
// OPERATIONS
// ============================================================================

/// How the selected polygons move together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtrudeMode {
    /// Each connected group moves along its average normal.
    Group,
    /// Each vertex moves along the average normal of its selected polygons.
    LocalNormal,
    /// Each polygon moves on its own, with walls between neighbours.
    ByPolygon,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PolyOpKind {
    Extrude { height: f32, mode: ExtrudeMode },
    /// Extrude, then grow (positive) or shrink (negative) the outline.
    Bevel   { height: f32, outline: f32, mode: ExtrudeMode },
    Inset   { amount: f32, by_polygon: bool },
    /// Move, rotate and scale the selection about its centre. `rotate` is a
    /// quaternion (x, y, z, w). Scale is applied first, then rotation.
    Transform { translate: [f32; 3], rotate: [f32; 4], scale: [f32; 3] },
    /// Delete the selection and the polygons that use it, leaving holes.
    Delete,
    /// Remove edges or vertices without leaving holes.
    Remove { clean: bool },
    /// Weld selected vertices closer than `threshold`.
    Weld { threshold: f32 },
    /// Collapse each connected part of the selection to a point.
    Collapse,
    /// New edges between selected edges (with `segments` cuts) or vertices.
    Connect { segments: u32 },
    /// Fill open borders.
    Cap,
    /// Join two borders, or two groups of polygons, with a band of quads.
    Bridge,
    /// Make the selected polygons a separate element.
    Detach,
    /// Give every polygon around the selected vertices its own copy.
    Break,
    Flip,
    /// Flatten onto a plane across X, Y or Z, or the best fitting one.
    MakePlanar { axis: Option<usize> },
    Relax { amount: f32, iterations: u32, hold_border: bool },
    /// Split the selected polygons into quads.
    Tessellate,
    /// Catmull-Clark subdivision of the whole mesh.
    Subdivide { iterations: u32 },
}

impl PolyOpKind {
    pub fn label(&self) -> &'static str {
        match self {
            PolyOpKind::Extrude { .. }    => "Extrude",
            PolyOpKind::Bevel { .. }      => "Bevel",
            PolyOpKind::Inset { .. }      => "Inset",
            PolyOpKind::Transform { .. }  => "Transform",
            PolyOpKind::Delete            => "Delete",
            PolyOpKind::Remove { .. }     => "Remove",
            PolyOpKind::Weld { .. }       => "Weld",
            PolyOpKind::Collapse          => "Collapse",
            PolyOpKind::Connect { .. }    => "Connect",
            PolyOpKind::Cap               => "Cap",
            PolyOpKind::Bridge            => "Bridge",
            PolyOpKind::Detach            => "Detach",
            PolyOpKind::Break             => "Break",
            PolyOpKind::Flip              => "Flip",
            PolyOpKind::MakePlanar { .. } => "Make planar",
            PolyOpKind::Relax { .. }      => "Relax",
            PolyOpKind::Tessellate        => "Tessellate",
            PolyOpKind::Subdivide { .. }  => "Subdivide",
        }
    }

    pub fn identity_transform() -> Self {
        PolyOpKind::Transform { translate: [0.0; 3], rotate: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] }
    }

    /// True when the operation leaves every vertex and polygon index alone,
    /// so the selection it used is still valid afterwards.
    pub fn keeps_indices(&self) -> bool {
        matches!(self, PolyOpKind::Transform { .. } | PolyOpKind::Flip
            | PolyOpKind::MakePlanar { .. } | PolyOpKind::Relax { .. })
    }
}

/// One entry of the Edit Poly node's operation list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolyOp {
    pub enabled:   bool,
    pub selection: PolySelection,
    pub kind:      PolyOpKind,
    /// Frozen: still applied, in order, but no longer listed for editing.
    #[serde(default)]
    pub collapsed: bool,
}

impl PolyOp {
    pub fn new(selection: PolySelection, kind: PolyOpKind) -> Self {
        Self { enabled: true, selection, kind, collapsed: false }
    }

    /// Apply the operation. Extrude, Bevel and Inset keep the indices of the
    /// selected polygons, so the same selection addresses the moved faces.
    pub fn apply(&self, mesh: &mut PolyMesh) {
        if !self.enabled { return; }
        let sel   = &self.selection;
        let level = sel.level.base();
        match &self.kind {
            PolyOpKind::Extrude { height, mode } => offset_faces(mesh, &sel.poly_mask(mesh), *height, 0.0, *mode),
            PolyOpKind::Bevel { height, outline, mode } => offset_faces(mesh, &sel.poly_mask(mesh), *height, -*outline, *mode),
            PolyOpKind::Inset { amount, by_polygon } => offset_faces(
                mesh, &sel.poly_mask(mesh), 0.0, *amount,
                if *by_polygon { ExtrudeMode::ByPolygon } else { ExtrudeMode::Group }),
            PolyOpKind::Transform { translate, rotate, scale } => mesh.transform_verts(
                &sel.vertex_set(mesh), Vec3::from_array(*translate),
                Quat::from_array(*rotate).normalize(), Vec3::from_array(*scale)),
            PolyOpKind::Delete => {
                let mask: Vec<bool> = match level {
                    SubLevel::Polygon => sel.poly_mask(mesh),
                    SubLevel::Vertex => {
                        let v = sel.resolve(mesh).verts;
                        mesh.polys.iter().map(|poly| poly.iter().any(|x| v[*x as usize])).collect()
                    }
                    _ => {
                        let e = sel.resolve(mesh).edges;
                        mesh.polys.iter().map(|poly| (0..poly.len())
                            .any(|i| e.contains(&edge_key(poly[i], poly[(i + 1) % poly.len()])))).collect()
                    }
                };
                mesh.delete_polys(&mask);
            }
            PolyOpKind::Remove { clean } => match level {
                SubLevel::Vertex  => mesh.remove_verts(&sel.resolve(mesh).verts),
                SubLevel::Polygon => {}
                _                 => mesh.remove_edges(&sel.resolve(mesh).edges, *clean),
            },
            PolyOpKind::Weld { threshold } => mesh.weld(&sel.vertex_set(mesh), *threshold),
            PolyOpKind::Collapse => mesh.collapse(&sel.vertex_set(mesh)),
            PolyOpKind::Connect { segments } => match level {
                SubLevel::Vertex => mesh.connect_verts(&sel.resolve(mesh).verts),
                _                => mesh.connect(&sel.edge_set(mesh), *segments),
            },
            PolyOpKind::Cap => match level {
                SubLevel::Edge => mesh.cap(Some(&sel.resolve(mesh).edges)),
                _              => mesh.cap(None),
            },
            PolyOpKind::Bridge => match level {
                SubLevel::Polygon => { mesh.bridge_polys(&sel.poly_mask(mesh)); }
                SubLevel::Edge    => { mesh.bridge_borders(Some(&sel.resolve(mesh).edges)); }
                _ => {}
            },
            PolyOpKind::Detach => mesh.detach(&sel.poly_mask(mesh)),
            PolyOpKind::Break  => mesh.break_verts(&sel.vertex_set(mesh)),
            PolyOpKind::Flip   => mesh.flip(&sel.poly_mask(mesh)),
            PolyOpKind::MakePlanar { axis } => mesh.make_planar(&sel.vertex_set(mesh), *axis),
            PolyOpKind::Relax { amount, iterations, hold_border } =>
                mesh.relax(&sel.vertex_set(mesh), *amount, (*iterations).min(200), *hold_border),
            PolyOpKind::Tessellate => mesh.tessellate(&sel.poly_mask(mesh)),
            PolyOpKind::Subdivide { iterations } => for _ in 0..(*iterations).min(4) { mesh.subdivide(); },
        }
    }
}

/// Mesh after the first `count` operations.
pub fn apply_ops(input: &PolyMesh, ops: &[PolyOp], count: usize) -> PolyMesh {
    let mut mesh = input.clone();
    for op in ops.iter().take(count) { op.apply(&mut mesh); }
    mesh
}

// ── Collapsing ───────────────────────────────────────────────────────────────

/// Add an operation. With `auto_collapse`, everything before it is collapsed.
pub fn push_op(ops: &mut Vec<PolyOp>, auto_collapse: bool, op: PolyOp) {
    if auto_collapse { collapse_all(ops); }
    ops.push(op);
}

/// Collapse one operation. A disabled one does nothing, so it is dropped.
pub fn collapse_op(ops: &mut Vec<PolyOp>, i: usize) {
    if i >= ops.len() { return; }
    if ops[i].enabled { ops[i].collapsed = true; } else { ops.remove(i); }
}

pub fn collapse_all(ops: &mut Vec<PolyOp>) {
    ops.retain(|op| op.enabled);
    for op in ops.iter_mut() { op.collapsed = true; }
}

/// Make the run of collapsed operations that contains `i` editable again.
pub fn restore_run(ops: &mut [PolyOp], i: usize) {
    if i >= ops.len() || !ops[i].collapsed { return; }
    let mut a = i;
    while a > 0 && ops[a - 1].collapsed { a -= 1; }
    let mut b = i;
    while b + 1 < ops.len() && ops[b + 1].collapsed { b += 1; }
    for op in &mut ops[a..=b] { op.collapsed = false; }
}

// ── Cached evaluation ────────────────────────────────────────────────────────

fn mesh_hash(mesh: &PolyMesh) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for v in &mesh.verts { for c in v.to_array() { c.to_bits().hash(&mut h); } }
    mesh.polys.hash(&mut h);
    h.finish()
}

fn ops_hash(seed: u64, ops: &[PolyOp]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut h);
    format!("{ops:?}").hash(&mut h);
    h.finish()
}

static CACHE: std::sync::Mutex<Vec<(u64, std::sync::Arc<PolyMesh>)>> = std::sync::Mutex::new(Vec::new());

fn cache_get(key: u64) -> Option<std::sync::Arc<PolyMesh>> {
    CACHE.lock().ok()?.iter().find(|(k, _)| *k == key).map(|(_, m)| m.clone())
}

fn cache_put(key: u64, mesh: std::sync::Arc<PolyMesh>) {
    if let Ok(mut c) = CACHE.lock() {
        if c.len() >= 12 { c.remove(0); }
        c.push((key, mesh));
    }
}

/// `apply_ops` with a memory. The same input and operations give the stored
/// mesh back, and the leading run of collapsed operations is stored on its
/// own, so editing a live operation does not replay the collapsed ones.
pub fn eval_cached(input: &PolyMesh, ops: &[PolyOp], count: usize) -> std::sync::Arc<PolyMesh> {
    let ops  = &ops[..count.min(ops.len())];
    let seed = mesh_hash(input);
    let key  = ops_hash(seed, ops);
    if let Some(m) = cache_get(key) { return m; }

    let frozen = ops.iter().take_while(|op| op.collapsed).count();
    let mesh = if frozen == 0 || frozen == ops.len() {
        apply_ops(input, ops, ops.len())
    } else {
        let fkey = ops_hash(seed, &ops[..frozen]);
        let base = cache_get(fkey).unwrap_or_else(|| {
            let m = std::sync::Arc::new(apply_ops(input, ops, frozen));
            cache_put(fkey, m.clone());
            m
        });
        let mut mesh = (*base).clone();
        for op in &ops[frozen..] { op.apply(&mut mesh); }
        mesh
    };
    let mesh = std::sync::Arc::new(mesh);
    cache_put(key, mesh.clone());
    mesh
}

/// Shared core of Extrude, Bevel and Inset.
///
/// The selected polygons are cut free along the border of the selection and
/// joined back with a ring of new quads. The freed vertices then move
/// `height` along a normal and `inset` inwards, across the border edges.
pub fn offset_faces(mesh: &mut PolyMesh, selected: &[bool], height: f32, inset: f32, mode: ExtrudeMode) {
    let sel: Vec<usize> = (0..mesh.polys.len())
        .filter(|p| selected.get(*p).copied().unwrap_or(false) && mesh.polys[*p].len() >= 3)
        .collect();
    if sel.is_empty() { return; }

    // Everything is measured on the mesh as it is before the operation.
    let base    = mesh.verts.clone();
    let normals: Vec<Vec3> = (0..mesh.polys.len()).map(|p| mesh.normal(p)).collect();
    let areas:   Vec<Vec3> = (0..mesh.polys.len()).map(|p| mesh.area_normal(p)).collect();

    // Regions are cut free separately: one per polygon, or the whole selection.
    let regions: Vec<Vec<usize>> = match mode {
        ExtrudeMode::ByPolygon => sel.iter().map(|p| vec![*p]).collect(),
        _ => vec![sel.clone()],
    };

    // Group mode: average normal of each connected group of selected polygons.
    let mut group_normal: HashMap<usize, Vec3> = HashMap::new();
    if mode == ExtrudeMode::Group {
        let mut by_edge: HashMap<[u32; 2], Vec<usize>> = HashMap::new();
        for p in &sel {
            let poly = &mesh.polys[*p];
            for i in 0..poly.len() { by_edge.entry(edge_key(poly[i], poly[(i + 1) % poly.len()])).or_default().push(*p); }
        }
        let mut group: HashMap<usize, usize> = HashMap::new();
        for start in &sel {
            if group.contains_key(start) { continue; }
            let mut members = vec![];
            let mut todo = vec![*start];
            group.insert(*start, *start);
            while let Some(p) = todo.pop() {
                members.push(p);
                let poly = &mesh.polys[p];
                for i in 0..poly.len() {
                    for q in &by_edge[&edge_key(poly[i], poly[(i + 1) % poly.len()])] {
                        if !group.contains_key(q) { group.insert(*q, *start); todo.push(*q); }
                    }
                }
            }
            let n = members.iter().map(|p| areas[*p]).sum::<Vec3>().normalize_or_zero();
            for p in members { group_normal.insert(p, n); }
        }
    }

    for region in regions {
        // Directed edges of the region, each with the polygon it belongs to.
        let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
        for p in &region {
            let poly = &mesh.polys[*p];
            for i in 0..poly.len() { directed.insert((poly[i], poly[(i + 1) % poly.len()]), *p); }
        }
        // Border: edges whose other side is not in the region.
        let border: Vec<((u32, u32), usize)> = region.iter().flat_map(|p| {
            let poly = &mesh.polys[*p];
            (0..poly.len()).map(move |i| ((poly[i], poly[(i + 1) % poly.len()]), *p)).collect::<Vec<_>>()
        }).filter(|((a, b), _)| !directed.contains_key(&(*b, *a))).collect();

        // Direction each vertex is raised along.
        let mut raise: HashMap<u32, Vec3> = HashMap::new();
        for p in &region {
            for v in &mesh.polys[*p] {
                let add = match mode {
                    ExtrudeMode::Group => group_normal.get(p).copied().unwrap_or(normals[*p]),
                    _ => normals[*p],
                };
                let e = raise.entry(*v).or_insert(Vec3::ZERO);
                if mode == ExtrudeMode::Group { *e = add; } else { *e += add; }
            }
        }

        // Inward shift of the border vertices. Each border edge pushes its
        // ends sideways, in the plane of its polygon; a vertex between two
        // border edges takes the shift that moves both edges by `inset`.
        let mut incoming: HashMap<u32, Vec3> = HashMap::new();
        let mut outgoing: HashMap<u32, Vec3> = HashMap::new();
        for ((a, b), p) in &border {
            let d = (base[*b as usize] - base[*a as usize]).normalize_or_zero();
            let inward = normals[*p].cross(d).normalize_or_zero();
            outgoing.insert(*a, inward);
            incoming.insert(*b, inward);
        }

        // Free the region: every vertex gets a copy that the region uses.
        let mut copy: HashMap<u32, u32> = HashMap::new();
        for p in &region {
            for v in mesh.polys[*p].clone() {
                if copy.contains_key(&v) { continue; }
                let mut pos = base[v as usize] + raise[&v].normalize_or_zero() * height;
                if let (Some(p1), Some(p2)) = (incoming.get(&v), outgoing.get(&v)) {
                    pos += (*p1 + *p2) / (1.0 + p1.dot(*p2)).max(0.25) * inset;
                }
                copy.insert(v, mesh.verts.len() as u32);
                mesh.verts.push(pos);
            }
        }
        for p in &region {
            for v in mesh.polys[*p].iter_mut() { *v = copy[v]; }
        }
        // Ring of quads between the old border and the freed one.
        for ((a, b), _) in &border {
            mesh.polys.push(vec![*a, *b, copy[b], copy[a]]);
        }
    }
    mesh.compact();
}

// ============================================================================
// PICKING
// ============================================================================

/// Camera as the picker needs it.
#[derive(Clone, Copy, Debug)]
pub struct PickView {
    /// World to clip space.
    pub view_proj: Mat4,
    /// Viewport size in pixels.
    pub size:      Vec2,
    /// Camera position.
    pub eye:       Vec3,
}

impl PickView {
    /// World position to pixels from the viewport's top-left. None behind the camera.
    pub fn project(&self, p: Vec3) -> Option<Vec2> {
        let clip = self.view_proj * p.extend(1.0);
        if clip.w <= 1e-6 { return None; }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new((ndc.x + 1.0) * 0.5 * self.size.x, (1.0 - ndc.y) * 0.5 * self.size.y))
    }

    /// Ray through a pixel: origin and unit direction.
    pub fn ray(&self, pixel: Vec2) -> (Vec3, Vec3) {
        let ndc = Vec2::new(pixel.x / self.size.x * 2.0 - 1.0, 1.0 - pixel.y / self.size.y * 2.0);
        let inv = self.view_proj.inverse();
        // Two depths along the pixel; works for any depth convention.
        let a = inv.project_point3(ndc.extend(0.25));
        let b = inv.project_point3(ndc.extend(0.75));
        let mut dir = (b - a).normalize_or_zero();
        if dir.dot(a - self.eye) < 0.0 { dir = -dir; }
        (self.eye, dir)
    }

    /// Polygons whose front side faces the camera.
    pub fn front_facing(&self, mesh: &PolyMesh) -> Vec<bool> {
        (0..mesh.polys.len())
            .map(|p| mesh.normal(p).dot(mesh.centroid(p) - self.eye) < 0.0)
            .collect()
    }
}

fn point_segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() < 1e-9 { 0.0 } else { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) };
    p.distance(a + ab * t)
}

fn ray_triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let h = d.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-9 { return None; }
    let s = o - a;
    let u = s.dot(h) / det;
    if !(0.0..=1.0).contains(&u) { return None; }
    let q = s.cross(e1);
    let v = d.dot(q) / det;
    if v < 0.0 || u + v > 1.0 { return None; }
    let t = e2.dot(q) / det;
    (t > 1e-6).then_some(t)
}

/// Nearest polygon hit by the ray through `pixel`, front faces only.
fn polygon_under(mesh: &PolyMesh, view: &PickView, pixel: Vec2) -> Option<(u32, f32)> {
    let (o, d) = view.ray(pixel);
    let front = view.front_facing(mesh);
    let mut best: Option<(u32, f32)> = None;
    for (p, poly) in mesh.polys.iter().enumerate() {
        if !front[p] { continue; }
        for i in 1..poly.len().saturating_sub(1) {
            let hit = ray_triangle(o, d,
                mesh.verts[poly[0] as usize], mesh.verts[poly[i] as usize], mesh.verts[poly[i + 1] as usize]);
            if let Some(t) = hit {
                if best.map(|b| t < b.1).unwrap_or(true) { best = Some((p as u32, t)); }
            }
        }
    }
    best
}

/// Vertices and edges that belong to at least one polygon facing the camera.
/// Components on the far side of the object are neither drawn nor picked.
pub fn visible_components(mesh: &PolyMesh, view: &PickView) -> (Vec<bool>, HashSet<[u32; 2]>) {
    let front = view.front_facing(mesh);
    let mut verts = vec![false; mesh.verts.len()];
    let mut edges = HashSet::new();
    for (p, poly) in mesh.polys.iter().enumerate() {
        if !front[p] { continue; }
        for i in 0..poly.len() {
            verts[poly[i] as usize] = true;
            edges.insert(edge_key(poly[i], poly[(i + 1) % poly.len()]));
        }
    }
    (verts, edges)
}

/// What a click at `pixel` selects at the given level. Vertices and edges
/// are picked within `radius` pixels.
pub fn pick_point(mesh: &PolyMesh, view: &PickView, level: SubLevel, pixel: Vec2, radius: f32) -> Option<Component> {
    let (vis_verts, vis_edges) = visible_components(mesh, view);
    match level {
        SubLevel::Polygon | SubLevel::Element => polygon_under(mesh, view, pixel).map(|(p, _)| Component::Polygon(p)),
        SubLevel::Vertex => {
            let mut best: Option<(u32, f32)> = None;
            for (v, pos) in mesh.verts.iter().enumerate() {
                if !vis_verts[v] { continue; }
                let Some(s) = view.project(*pos) else { continue };
                let d = s.distance(pixel);
                if d <= radius && best.map(|b| d < b.1).unwrap_or(true) { best = Some((v as u32, d)); }
            }
            best.map(|(v, _)| Component::Vertex(v))
        }
        SubLevel::Edge | SubLevel::Border => {
            let open = (level == SubLevel::Border).then(|| mesh.open_edges());
            let mut best: Option<([u32; 2], f32)> = None;
            for e in &vis_edges {
                if open.as_ref().map(|o| !o.contains(e)).unwrap_or(false) { continue; }
                let (Some(a), Some(b)) = (view.project(mesh.verts[e[0] as usize]), view.project(mesh.verts[e[1] as usize])) else { continue };
                let d = point_segment_distance(pixel, a, b);
                if d <= radius && best.map(|b| d < b.1).unwrap_or(true) { best = Some((*e, d)); }
            }
            best.map(|(e, _)| Component::Edge(e))
        }
    }
}

/// Everything a box drawn between two pixels selects. A component counts
/// when all of its vertices are inside the box.
pub fn pick_rect(mesh: &PolyMesh, view: &PickView, level: SubLevel, a: Vec2, b: Vec2) -> Vec<Component> {
    let (min, max) = (a.min(b), a.max(b));
    let inside: Vec<bool> = mesh.verts.iter()
        .map(|p| view.project(*p).map(|s| s.cmpge(min).all() && s.cmple(max).all()).unwrap_or(false))
        .collect();
    let (vis_verts, vis_edges) = visible_components(mesh, view);
    match level {
        SubLevel::Vertex => (0..mesh.verts.len())
            .filter(|v| vis_verts[*v] && inside[*v])
            .map(|v| Component::Vertex(v as u32)).collect(),
        SubLevel::Edge | SubLevel::Border => {
            let open = (level == SubLevel::Border).then(|| mesh.open_edges());
            let mut edges: Vec<[u32; 2]> = vis_edges.into_iter()
                .filter(|e| inside[e[0] as usize] && inside[e[1] as usize])
                .filter(|e| open.as_ref().map(|o| o.contains(e)).unwrap_or(true)).collect();
            edges.sort();
            edges.into_iter().map(Component::Edge).collect()
        }
        SubLevel::Polygon | SubLevel::Element => {
            let front = view.front_facing(mesh);
            (0..mesh.polys.len())
                .filter(|p| front[*p] && mesh.polys[*p].iter().all(|v| inside[*v as usize]))
                .map(|p| Component::Polygon(p as u32)).collect()
        }
    }
}

/// Widen picks to what the level selects as a unit: a whole border at
/// Border level, a whole element at Element level.
pub fn widen_pick(mesh: &PolyMesh, level: SubLevel, picked: Vec<Component>) -> Vec<Component> {
    match level {
        SubLevel::Border => {
            let hit: HashSet<[u32; 2]> = picked.iter()
                .filter_map(|c| if let Component::Edge(e) = c { Some(edge_key(e[0], e[1])) } else { None }).collect();
            let mut out = vec![];
            for lp in mesh.border_loops() {
                let edges: Vec<[u32; 2]> = (0..lp.len()).map(|i| edge_key(lp[i], lp[(i + 1) % lp.len()])).collect();
                if edges.iter().any(|e| hit.contains(e)) { out.extend(edges.into_iter().map(Component::Edge)); }
            }
            out
        }
        SubLevel::Element => {
            let element = mesh.elements();
            let hit: HashSet<usize> = picked.iter()
                .filter_map(|c| if let Component::Polygon(p) = c { element.get(*p as usize).copied() } else { None }).collect();
            (0..mesh.polys.len()).filter(|p| hit.contains(&element[*p])).map(|p| Component::Polygon(p as u32)).collect()
        }
        _ => picked,
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> PolyMesh { PolyMesh::from_mesh(&crate::node_graph::nodes::create_cube(1.0)) }
    fn grid(n: u32) -> PolyMesh { PolyMesh::from_mesh(&crate::node_graph::nodes::create_grid(n, n, n as f32)) }

    fn top() -> PolySelection {
        PolySelection { source: SelSource::ByNormal { dir: [0.0, 1.0, 0.0], angle: 5.0 }, ..Default::default() }
    }
    fn op(selection: PolySelection, kind: PolyOpKind) -> PolyOp { PolyOp::new(selection, kind) }
    fn area(m: &PolyMesh, p: usize) -> f32 { m.area_normal(p).length() * 0.5 }

    #[test]
    fn primitives_face_outwards() {
        // Sphere: every polygon and every drawn triangle points away from the centre.
        let md = crate::node_graph::nodes::create_sphere(1.0, 16);
        let s = PolyMesh::from_mesh(&md);
        assert!(s.volume() > 3.9 && s.volume() < 4.19, "{}", s.volume());
        for p in 0..s.polys.len() {
            if s.area_normal(p).length() < 1e-6 { continue; }
            assert!(s.normal(p).dot(s.centroid(p)) > 0.0, "polygon {p}");
        }
        for t in md.indices.chunks_exact(3) {
            let [a, b, c] = [0, 1, 2].map(|k| Vec3::from_array(md.vertices[t[k] as usize]));
            let n = (b - a).cross(c - a);
            if n.length() < 1e-7 { continue; }
            assert!(n.dot(a + b + c) > 0.0);
        }
        for (v, n) in md.vertices.iter().zip(&md.normals) {
            assert!(Vec3::from_array(*n).dot(Vec3::from_array(*v)) > 0.99, "{n:?} at {v:?}");
        }
        // Grid: faces up.
        let g = grid(3);
        for p in 0..g.polys.len() { assert!(g.normal(p).y > 0.99); }
    }

    #[test]
    fn cube_is_six_outward_quads() {
        let c = cube();
        assert_eq!((c.verts.len(), c.polys.len()), (8, 6));
        assert!(c.polys.iter().all(|p| p.len() == 4));
        assert!(c.is_closed());
        assert!((c.volume() - 1.0).abs() < 1e-5);
        for p in 0..6 { assert!(c.normal(p).dot(c.centroid(p)) > 0.49); }
        assert_eq!(c.edges().len(), 12);
    }

    #[test]
    fn extrude_one_face() {
        let mut m = cube();
        op(top(), PolyOpKind::Extrude { height: 0.5, mode: ExtrudeMode::Group }).apply(&mut m);
        assert_eq!((m.verts.len(), m.polys.len()), (12, 10));
        assert!(m.is_closed());
        assert!((m.volume() - 1.5).abs() < 1e-5);
        // The selected polygon keeps its index and now sits at the new height.
        let sel = top().poly_mask(&m);
        let p = sel.iter().position(|s| *s).unwrap();
        assert_eq!(sel.iter().filter(|s| **s).count(), 1);
        assert!((m.centroid(p).y - 1.0).abs() < 1e-5);
        assert!((area(&m, p) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn group_extrude_has_no_wall_between_neighbours() {
        // Two adjacent cells of a 3x3 grid.
        let mut m = grid(3);
        let sel = PolySelection::picked_polys(vec![0, 1]);
        let mut by_poly = m.clone();
        op(sel.clone(), PolyOpKind::Extrude { height: 1.0, mode: ExtrudeMode::Group }).apply(&mut m);
        // 6 border edges -> 6 walls, 6 new vertices.
        assert_eq!(m.polys.len(), 9 + 6);
        assert_eq!(m.verts.len(), 16 + 6);
        assert!((m.centroid(0).y - 1.0).abs() < 1e-5 && (m.centroid(1).y - 1.0).abs() < 1e-5);
        // By polygon: each cell gets its own four walls.
        op(sel, PolyOpKind::Extrude { height: 1.0, mode: ExtrudeMode::ByPolygon }).apply(&mut by_poly);
        assert_eq!(by_poly.polys.len(), 9 + 8);
        assert_eq!(by_poly.verts.len(), 16 + 8);
    }

    #[test]
    fn extrude_whole_closed_mesh_offsets_it() {
        // Every face selected: no border, so local-normal extrude just moves the surface.
        let mut m = cube();
        let all = PolySelection { source: SelSource::All, ..Default::default() };
        op(all, PolyOpKind::Extrude { height: 0.1, mode: ExtrudeMode::LocalNormal }).apply(&mut m);
        assert_eq!((m.verts.len(), m.polys.len()), (8, 6));
        assert!(m.is_closed());
        assert!(m.volume() > 1.0);
    }

    #[test]
    fn by_polygon_extrude_of_all_faces() {
        let mut m = cube();
        let all = PolySelection { source: SelSource::All, ..Default::default() };
        op(all, PolyOpKind::Extrude { height: 1.0, mode: ExtrudeMode::ByPolygon }).apply(&mut m);
        // Six unit boxes on a unit cube: a plus shape in 3D.
        assert_eq!(m.polys.len(), 6 + 24);
        assert!(m.is_closed());
        assert!((m.volume() - 7.0).abs() < 1e-4);
    }

    #[test]
    fn inset_keeps_surface_and_shrinks_face() {
        let mut m = cube();
        op(top(), PolyOpKind::Inset { amount: 0.25, by_polygon: false }).apply(&mut m);
        assert_eq!((m.verts.len(), m.polys.len()), (12, 10));
        assert!(m.is_closed());
        assert!((m.volume() - 1.0).abs() < 1e-5);
        let p = top().poly_mask(&m).iter().position(|s| *s).unwrap();
        // 1 x 1 face inset by 0.25 on every side: 0.5 x 0.5.
        assert!((area(&m, p) - 0.25).abs() < 1e-5, "{}", area(&m, p));
        assert!((m.centroid(p).y - 0.5).abs() < 1e-6);
    }

    #[test]
    fn group_inset_follows_the_outline() {
        // 2 x 1 block of cells on a grid, inset as one: 1.5 x 0.5 remains.
        let mut m = grid(3);
        op(PolySelection::picked_polys(vec![0, 1]), PolyOpKind::Inset { amount: 0.25, by_polygon: false }).apply(&mut m);
        assert!((area(&m, 0) + area(&m, 1) - 1.5 * 0.5).abs() < 1e-5);
        // Separately: two 0.5 x 0.5 squares.
        let mut m = grid(3);
        op(PolySelection::picked_polys(vec![0, 1]), PolyOpKind::Inset { amount: 0.25, by_polygon: true }).apply(&mut m);
        assert!((area(&m, 0) - 0.25).abs() < 1e-5 && (area(&m, 1) - 0.25).abs() < 1e-5);
    }

    #[test]
    fn bevel_raises_and_scales() {
        let mut m = cube();
        op(top(), PolyOpKind::Bevel { height: 0.5, outline: -0.25, mode: ExtrudeMode::Group }).apply(&mut m);
        assert!(m.is_closed());
        let p = top().poly_mask(&m).iter().position(|s| *s).unwrap();
        assert!((m.centroid(p).y - 1.0).abs() < 1e-5);
        assert!((area(&m, p) - 0.25).abs() < 1e-5);
        // Frustum on a unit cube: 1 + h/3 * (A1 + A2 + sqrt(A1*A2)).
        let expect = 1.0 + 0.5 / 3.0 * (1.0 + 0.25 + 0.5);
        assert!((m.volume() - expect).abs() < 1e-4, "{} vs {expect}", m.volume());
        // Positive outline flares out.
        let mut m = cube();
        op(top(), PolyOpKind::Bevel { height: 0.5, outline: 0.25, mode: ExtrudeMode::Group }).apply(&mut m);
        let p = top().poly_mask(&m).iter().position(|s| *s).unwrap();
        assert!((area(&m, p) - 2.25).abs() < 1e-4);
    }

    #[test]
    fn operations_chain_on_the_same_selection() {
        // Inset, extrude, bevel on the picked top face: the usual way to pull
        // a tower out of a box. The face keeps its index through every step.
        let top_face = PolySelection::picked_polys(vec![3]);
        assert_eq!(cube().normal(3), Vec3::Y);
        let ops = vec![
            op(top_face.clone(), PolyOpKind::Inset { amount: 0.1, by_polygon: false }),
            op(top_face.clone(), PolyOpKind::Extrude { height: 0.5, mode: ExtrudeMode::Group }),
            op(top_face.clone(), PolyOpKind::Bevel { height: 0.2, outline: -0.1, mode: ExtrudeMode::Group }),
        ];
        let m = apply_ops(&cube(), &ops, 3);
        assert!(m.is_closed());
        assert_eq!(m.polys.len(), 6 + 12);
        assert!((m.centroid(3).y - 1.2).abs() < 1e-5);
        assert!((area(&m, 3) - 0.6 * 0.6).abs() < 1e-5);          // 1 - 2*0.1 - 2*0.1 per side
        // Box, plus a 0.8 x 0.8 x 0.5 column, plus a frustum from 0.8 to 0.6.
        let expect = 1.0 + 0.64 * 0.5 + 0.2 / 3.0 * (0.64 + 0.36 + 0.48);
        assert!((m.volume() - expect).abs() < 1e-4, "{} vs {expect}", m.volume());
        // Stopping early gives the intermediate mesh; a disabled step is skipped.
        assert_eq!(apply_ops(&cube(), &ops, 1).polys.len(), 10);
        let mut skip = ops.clone();
        skip[1].enabled = false;
        assert!((apply_ops(&cube(), &skip, 3).centroid(3).y - 0.7).abs() < 1e-5);
    }

    #[test]
    fn selection_rules() {
        let g = grid(4);     // 4 x 4 cells from -2 to 2
        let all = PolySelection { source: SelSource::All, ..Default::default() };
        assert_eq!(all.count(&g), 16);
        let corner = PolySelection::picked_polys(vec![0]);
        assert_eq!(PolySelection { grow: 1, ..corner.clone() }.count(&g), 4);
        assert_eq!(PolySelection { grow: 2, ..corner.clone() }.count(&g), 9);
        assert_eq!(PolySelection { invert: true, ..corner.clone() }.count(&g), 15);
        assert_eq!(PolySelection { grow: -1, ..all.clone() }.count(&g), 16);   // open grid: nothing outside to shrink from
        let inner = PolySelection { source: SelSource::InBox { min: [-1.0, -1.0, -1.0], max: [1.0, 1.0, 1.0] }, ..Default::default() };
        assert_eq!(inner.count(&g), 4);
        assert_eq!(PolySelection { grow: -1, ..PolySelection { grow: 0, ..inner.clone() } }.count(&g), 0);

        let c = cube();
        assert_eq!(top().count(&c), 1);
        let upper = PolySelection { source: SelSource::ByNormal { dir: [0.0, 1.0, 0.0], angle: 95.0 }, ..Default::default() };
        assert_eq!(upper.count(&c), 5);
        // Vertex and edge levels.
        let v = PolySelection { level: SubLevel::Vertex, ..top() };
        assert_eq!(v.count(&c), 4);
        assert_eq!(v.poly_mask(&c).iter().filter(|s| **s).count(), 1);
        assert_eq!(PolySelection { grow: 1, ..v.clone() }.count(&c), 8);
        let e = PolySelection { level: SubLevel::Edge, ..top() };
        assert_eq!(e.count(&c), 4);
        assert_eq!(e.poly_mask(&c).iter().filter(|s| **s).count(), 1);
        assert_eq!(PolySelection { grow: 1, ..e.clone() }.count(&c), 8);
        assert_eq!(PolySelection { invert: true, ..e }.count(&c), 8);
    }

    fn camera(eye: Vec3) -> PickView {
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(45f32.to_radians(), 800.0 / 600.0, 0.1, 100.0);
        PickView { view_proj: proj * view, size: Vec2::new(800.0, 600.0), eye }
    }

    #[test]
    fn click_picks_the_face_edge_and_vertex_in_front() {
        let c = cube();
        let view = camera(Vec3::new(0.0, 0.0, 5.0));
        let centre = Vec2::new(400.0, 300.0);
        // Looking down -Z at the +Z face.
        let Some(Component::Polygon(p)) = pick_point(&c, &view, SubLevel::Polygon, centre, 8.0) else { panic!("no face") };
        assert!(c.normal(p as usize).z > 0.9);
        assert_eq!(pick_point(&c, &view, SubLevel::Polygon, Vec2::new(10.0, 10.0), 8.0), None);

        // A corner of the front face.
        let corner = Vec3::new(0.5, 0.5, 0.5);
        let px = view.project(corner).unwrap();
        let Some(Component::Vertex(v)) = pick_point(&c, &view, SubLevel::Vertex, px + Vec2::new(3.0, -2.0), 8.0) else { panic!("no vertex") };
        assert!((c.verts[v as usize] - corner).length() < 1e-6);
        assert_eq!(pick_point(&c, &view, SubLevel::Vertex, centre, 8.0), None);

        // Middle of the front face's top edge.
        let mid = view.project(Vec3::new(0.0, 0.5, 0.5)).unwrap();
        let Some(Component::Edge(e)) = pick_point(&c, &view, SubLevel::Edge, mid + Vec2::new(0.0, 2.0), 6.0) else { panic!("no edge") };
        let (a, b) = (c.verts[e[0] as usize], c.verts[e[1] as usize]);
        assert!(a.y > 0.4 && b.y > 0.4 && a.z > 0.4 && b.z > 0.4);
    }

    #[test]
    fn box_select_ignores_the_far_side() {
        let c = cube();
        let view = camera(Vec3::new(0.0, 0.0, 5.0));
        let (a, b) = (Vec2::ZERO, view.size);
        // Head on: only the front face and its components face the camera.
        assert_eq!(pick_rect(&c, &view, SubLevel::Polygon, a, b).len(), 1);
        assert_eq!(pick_rect(&c, &view, SubLevel::Vertex, a, b).len(), 4);
        assert_eq!(pick_rect(&c, &view, SubLevel::Edge, a, b).len(), 4);
        // From a corner: three faces, seven vertices, nine edges.
        let view = camera(Vec3::new(4.0, 4.0, 4.0));
        assert_eq!(pick_rect(&c, &view, SubLevel::Polygon, a, b).len(), 3);
        assert_eq!(pick_rect(&c, &view, SubLevel::Vertex, a, b).len(), 7);
        assert_eq!(pick_rect(&c, &view, SubLevel::Edge, a, b).len(), 9);
        // A small box around one projected vertex.
        let px = view.project(Vec3::splat(0.5)).unwrap();
        assert_eq!(pick_rect(&c, &view, SubLevel::Vertex, px - Vec2::splat(4.0), px + Vec2::splat(4.0)).len(), 1);
    }

    #[test]
    fn clicks_edit_a_selection() {
        let c = cube();
        let mut s = PolySelection::default();
        s.apply_pick(&c, &[Component::Polygon(2)], PickMode::Replace);
        s.apply_pick(&c, &[Component::Polygon(4), Component::Polygon(2)], PickMode::Add);
        assert_eq!(s.polys, vec![2, 4]);
        s.apply_pick(&c, &[Component::Polygon(2)], PickMode::Remove);
        assert_eq!(s.polys, vec![4]);
        s.apply_pick(&c, &[], PickMode::Replace);
        assert!(s.polys.is_empty());

        // Clicking on a rule-based selection first turns it into picked components.
        let mut s = PolySelection { grow: 0, ..top() };
        let before = s.poly_mask(&c);
        s.apply_pick(&c, &[], PickMode::Add);
        assert_eq!(s.source, SelSource::Picked);
        assert_eq!(s.poly_mask(&c), before);

        // Each level keeps its own picks.
        let mut s = PolySelection { level: SubLevel::Edge, ..Default::default() };
        s.apply_pick(&c, &[Component::Edge([3, 1]), Component::Polygon(0)], PickMode::Replace);
        assert_eq!(s.edges, vec![[1, 3]]);
        assert!(s.polys.is_empty());
    }

    #[test]
    fn stale_picks_are_ignored() {
        // Indices beyond the mesh (after an upstream change) must not panic.
        let c = cube();
        let s = PolySelection { polys: vec![2, 99], verts: vec![500], edges: vec![[0, 77]], ..Default::default() };
        assert_eq!(s.count(&c), 1);
        assert_eq!(PolySelection { level: SubLevel::Vertex, ..s.clone() }.count(&c), 0);
        assert_eq!(PolySelection { level: SubLevel::Edge, ..s.clone() }.count(&c), 0);
        let mut m = c.clone();
        op(s, PolyOpKind::Extrude { height: 1.0, mode: ExtrudeMode::Group }).apply(&mut m);
        assert!(m.is_closed());
    }

    #[test]
    fn round_trip_through_mesh_data() {
        let mut m = cube();
        op(top(), PolyOpKind::Extrude { height: 0.5, mode: ExtrudeMode::Group }).apply(&mut m);
        let md = m.to_mesh();
        assert_eq!(md.face_count, 10);
        assert_eq!(md.indices.len(), 10 * 2 * 3);
        assert_eq!(PolyMesh::from_mesh(&md), m);
        // A mesh without polygon data is read as triangles.
        let tri = MeshData::from_triangles(md.vertices.clone(), md.indices.clone());
        assert_eq!(PolyMesh::from_mesh(&tri).polys.len(), 20);
    }
}
