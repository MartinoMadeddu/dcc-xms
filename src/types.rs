
// ============================================================================

use std::collections::HashMap;
use std::fmt;
use bevy::prelude::*;
use bevy_egui::egui;

// ============================================================================
// BEVY COMPONENTS
// ============================================================================

#[derive(Component)]
pub struct MainCamera;

#[derive(Component)]
pub struct GeneratedMesh;

#[derive(Component)]
pub struct GroundGrid;

#[derive(Resource, Default)]
pub struct ViewportRect(pub Option<egui::Rect>);

// ============================================================================
// SHARED IDs
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubnetId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneObjectId(pub usize);

// ============================================================================
// ATTRIBUTE INTERPOLATION  (unchanged — maps directly to USD)
// ============================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub enum PrimVarInterp {
    #[default]
    Vertex,       // one value per vertex/point
    Uniform,      // one value per face
    FaceVarying,  // one value per face-vertex
    Constant,     // one value for the whole prim
}

// ============================================================================
// TYPED ATTRIBUTE  — the core new abstraction
// ============================================================================

/// A named, typed data stream.
/// T can be f32, Vec2, Vec3, Vec4, Quat, i32, bool — anything Clone+Debug.
/// Nodes operate on Attribute<T> directly, giving you full type safety.
#[derive(Clone, Debug)]
pub struct Attribute<T: Clone + fmt::Debug> {
    pub name:   String,
    pub interp: PrimVarInterp,
    pub values: Vec<T>,
}

impl<T: Clone + fmt::Debug + Default> Default for Attribute<T> {
    fn default() -> Self {
        Self { name: String::new(), interp: PrimVarInterp::Vertex, values: Vec::new() }
    }
}

impl<T: Clone + fmt::Debug> Attribute<T> {
    pub fn new(name: impl Into<String>, interp: PrimVarInterp, values: Vec<T>) -> Self {
        Self { name: name.into(), interp, values }
    }

    /// Convenience: create a vertex-interpolated attribute
    pub fn vertex(name: impl Into<String>, values: Vec<T>) -> Self {
        Self::new(name, PrimVarInterp::Vertex, values)
    }

    /// Convenience: create a uniform (per-face) attribute
    pub fn uniform(name: impl Into<String>, values: Vec<T>) -> Self {
        Self::new(name, PrimVarInterp::Uniform, values)
    }

    pub fn len(&self) -> usize      { self.values.len() }
    pub fn is_empty(&self) -> bool  { self.values.is_empty() }
}

// ============================================================================
// TYPE-ERASED ATTRIBUTE  — for dynamic channels stored in MeshData.attributes
// ============================================================================

/// Owns a typed attribute without knowing T at compile time.
/// Used for primvars, UVs, density, colour — anything beyond positions/normals.
#[derive(Clone, Debug)]
pub enum AnyAttribute {
    Float(Attribute<f32>),
    Vec2(Attribute<Vec2>),
    Vec3(Attribute<Vec3>),
    Vec4(Attribute<Vec4>),
    Quat(Attribute<Quat>),
    Int(Attribute<i32>),
    Bool(Attribute<bool>),
}

impl AnyAttribute {
    pub fn name(&self) -> &str {
        match self {
            Self::Float(a) => &a.name,
            Self::Vec2(a)  => &a.name,
            Self::Vec3(a)  => &a.name,
            Self::Vec4(a)  => &a.name,
            Self::Quat(a)  => &a.name,
            Self::Int(a)   => &a.name,
            Self::Bool(a)  => &a.name,
        }
    }

    pub fn interp(&self) -> &PrimVarInterp {
        match self {
            Self::Float(a) => &a.interp,
            Self::Vec2(a)  => &a.interp,
            Self::Vec3(a)  => &a.interp,
            Self::Vec4(a)  => &a.interp,
            Self::Quat(a)  => &a.interp,
            Self::Int(a)   => &a.interp,
            Self::Bool(a)  => &a.interp,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Float(a) => a.len(),
            Self::Vec2(a)  => a.len(),
            Self::Vec3(a)  => a.len(),
            Self::Vec4(a)  => a.len(),
            Self::Quat(a)  => a.len(),
            Self::Int(a)   => a.len(),
            Self::Bool(a)  => a.len(),
        }
    }

    /// Human-readable type string — used by the prim inspector
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Float(_) => "float",
            Self::Vec2(_)  => "float2",
            Self::Vec3(_)  => "float3",
            Self::Vec4(_)  => "float4",
            Self::Quat(_)  => "quatf",
            Self::Int(_)   => "int",
            Self::Bool(_)  => "bool",
        }
    }

    // Typed downcasts
    pub fn as_float(&self) -> Option<&Attribute<f32>>  { if let Self::Float(a) = self { Some(a) } else { None } }
    pub fn as_vec2(&self)  -> Option<&Attribute<Vec2>> { if let Self::Vec2(a)  = self { Some(a) } else { None } }
    pub fn as_vec3(&self)  -> Option<&Attribute<Vec3>> { if let Self::Vec3(a)  = self { Some(a) } else { None } }
    pub fn as_vec4(&self)  -> Option<&Attribute<Vec4>> { if let Self::Vec4(a)  = self { Some(a) } else { None } }
    pub fn as_quat(&self)  -> Option<&Attribute<Quat>> { if let Self::Quat(a)  = self { Some(a) } else { None } }
    pub fn as_int(&self)   -> Option<&Attribute<i32>>  { if let Self::Int(a)   = self { Some(a) } else { None } }
    pub fn as_bool(&self)  -> Option<&Attribute<bool>> { if let Self::Bool(a)  = self { Some(a) } else { None } }
}

// ============================================================================
// MESH DATA
// ============================================================================

#[derive(Clone, Debug, Default)]
pub struct MeshData {
    /// Primary position stream ("P" in Houdini / USD parlance)
    pub positions:  Attribute<Vec3>,
    /// Triangle indices into positions
    pub indices:    Vec<u32>,
    /// Per-vertex normals — None until compute_normals() is called
    pub normals:    Option<Attribute<Vec3>>,
    /// Scatter / point-cloud positions (separate from mesh positions)
    pub points:     Vec<Vec3>,
    /// All other named data streams: UVs, density, colour, custom, etc.
    pub attributes: HashMap<String, AnyAttribute>,
    /// Polygon count before triangulation (for display in inspector)
    pub face_count: usize,
}

impl MeshData {
    /// Primary constructor — same call-site signature as before.
    /// vertices: Vec<[f32;3]> is accepted and converted to Attribute<Vec3>.
    pub fn from_triangles(vertices: Vec<[f32; 3]>, indices: Vec<u32>) -> Self {
        let face_count = indices.len() / 3;
        let positions  = Attribute::vertex(
            "P",
            vertices.into_iter().map(Vec3::from).collect(),
        );
        Self { positions, indices, face_count, ..Default::default() }
    }

    // ── Backward-compat helpers (used by main.rs, usd_loader.rs) ─────────────

    /// Positions as [f32;3] arrays — for Bevy mesh upload and legacy code.
    pub fn positions_as_arrays(&self) -> Vec<[f32; 3]> {
        self.positions.values.iter().map(|v| v.to_array()).collect()
    }

    /// Normals as [f32;3] arrays — falls back to up-vector if not computed.
    pub fn normals_as_arrays(&self) -> Vec<[f32; 3]> {
        match &self.normals {
            Some(n) => n.values.iter().map(|v| v.to_array()).collect(),
            None    => self.positions.values.iter().map(|_| [0.0f32, 1.0, 0.0]).collect(),
        }
    }

    /// Mutable slice over position values — replaces `mesh.vertices.iter_mut()`
    pub fn positions_mut(&mut self) -> &mut Vec<Vec3> {
        &mut self.positions.values
    }

    pub fn vertex_count(&self) -> usize { self.positions.len() }

    // ── Attribute access ──────────────────────────────────────────────────────

    pub fn get_attribute(&self, name: &str) -> Option<&AnyAttribute> {
        self.attributes.get(name)
    }

    pub fn set_attribute(&mut self, attr: AnyAttribute) {
        self.attributes.insert(attr.name().to_string(), attr);
    }

    pub fn remove_attribute(&mut self, name: &str) {
        self.attributes.remove(name);
    }

    // ── Normal computation ────────────────────────────────────────────────────

    pub fn compute_normals(&mut self) {
        let n = self.positions.len();
        let mut normals = vec![Vec3::ZERO; n];
        let mut counts  = vec![0u32; n];

        for tri in self.indices.chunks(3) {
            if tri.len() < 3 { continue; }
            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            if a >= n || b >= n || c >= n { continue; }
            let va = self.positions.values[a];
            let vb = self.positions.values[b];
            let vc = self.positions.values[c];
            let face_n = (vb - va).cross(vc - va);
            normals[a] += face_n; counts[a] += 1;
            normals[b] += face_n; counts[b] += 1;
            normals[c] += face_n; counts[c] += 1;
        }

        let computed: Vec<Vec3> = normals.iter().zip(&counts)
            .map(|(n, &c)| if c > 0 { n.normalize() } else { Vec3::Y })
            .collect();

        // Store as named attribute so the inspector can display it
        self.attributes.insert(
            "N".into(),
            AnyAttribute::Vec3(Attribute::vertex("N", computed.clone())),
        );
        self.normals = Some(Attribute::vertex("N", computed));
    }

    pub fn num_faces(&self) -> usize         { self.face_count }
    pub fn num_face_varying(&self) -> usize  { self.indices.len() }

    pub fn ensure_standard_attributes(&mut self) {
        if self.attributes.contains_key("ptIndex") { return; } // already done

        let pt_count  = self.positions.len();
        let tri_count = self.indices.len() / 3;

        // index/count attributes
        self.set_attribute(AnyAttribute::Int(
            Attribute::vertex("ptIndex",    (0..pt_count  as i32).collect())));
        self.set_attribute(AnyAttribute::Int(
            Attribute::vertex("primIndex",  (0..tri_count as i32).collect())));
        self.set_attribute(AnyAttribute::Int(
            Attribute::vertex("ptsNumber",  vec![pt_count  as i32; pt_count])));
        self.set_attribute(AnyAttribute::Int(
            Attribute::vertex("primsNumber", vec![tri_count as i32; pt_count])));

        // N — compute_normals() already stores it in attributes, just call it if missing
        if self.normals.is_none() {
            self.compute_normals(); // also writes "N" into attributes for free
        }

        // uv — parametric fallback only if not already loaded from USD/source
        if !self.attributes.contains_key("uv") {
            let uvs = (0..pt_count).map(|i| {
                let t = if pt_count > 1 { i as f32 / (pt_count - 1) as f32 } else { 0.0 };
                Vec2::new(t, t)
            }).collect();
            self.set_attribute(AnyAttribute::Vec2(Attribute::vertex("uv", uvs)));
        }
    }
}

// ============================================================================
// OUTER NODE TYPES
// ============================================================================

#[derive(Clone, Debug)]
pub enum NodeType {
    CreateCube   { size: f32 },
    CreateSphere { radius: f32, segments: u32 },
    CreateGrid   { rows: u32, cols: u32, size: f32 },
    LoadUsd      { path: String },
    Transform    { translation: Vec3, rotation: Vec3, scale: Vec3 },
    Merge,
    ScatterPoints { count: u32, seed: u32 },
    CopyToPoints,
    Subnet       { id: SubnetId, name: String },
    Output,
}

pub fn node_type_icon(t: &NodeType) -> &'static str {
    match t {
        NodeType::CreateCube { .. }    => "◼",
        NodeType::CreateSphere { .. }  => "●",
        NodeType::CreateGrid { .. }    => "⊞",
        NodeType::LoadUsd { .. }       => "📂",
        NodeType::Transform { .. }     => "⟲",
        NodeType::Merge                => "⊕",
        NodeType::ScatterPoints { .. } => "⁙",
        NodeType::CopyToPoints         => "❇",
        NodeType::Subnet { .. }        => "▣",
        NodeType::Output               => "▶",
    }
}

pub fn node_type_label(t: &NodeType) -> &'static str {
    match t {
        NodeType::CreateCube { .. }    => "Create Cube",
        NodeType::CreateSphere { .. }  => "Create Sphere",
        NodeType::CreateGrid { .. }    => "Create Grid",
        NodeType::LoadUsd { .. }       => "Load USD",
        NodeType::Transform { .. }     => "Transform",
        NodeType::Merge                => "Merge",
        NodeType::ScatterPoints { .. } => "Scatter Points",
        NodeType::CopyToPoints         => "Copy to Points",
        NodeType::Subnet { .. }        => "Subnet",
        NodeType::Output               => "Output",
    }
}

// ============================================================================
// SUBNET NODE TYPES
// ============================================================================

#[derive(Clone, Debug)]
pub enum SubnetNodeType {
    SubInput,
    SubOutput,
    AddVec3,
    SubtractVec3,
    MultiplyVec3 { scalar: f32 },
    CrossProduct,
    Normalize,
    DotProduct,
    LerpVec3     { t: f32 },
    ConstVec3    { value: Vec3 },
    ConstFloat   { value: f32  },
    ConstInt     { value: i32  },
    ScatterPoints { count: u32, seed: u32 },
    GetTemplate,
    GetAttribute { target: GetAttributeTarget },
    CopyToPoints,
}

pub fn subnet_node_icon(t: &SubnetNodeType) -> &'static str {
    match t {
        SubnetNodeType::SubInput             => "▶",
        SubnetNodeType::SubOutput            => "◀",
        SubnetNodeType::AddVec3              => "＋",
        SubnetNodeType::SubtractVec3         => "－",
        SubnetNodeType::MultiplyVec3 { .. }  => "✕",
        SubnetNodeType::CrossProduct         => "×",
        SubnetNodeType::Normalize            => "|v|",
        SubnetNodeType::DotProduct           => "·",
        SubnetNodeType::LerpVec3 { .. }      => "≈",
        SubnetNodeType::ConstVec3 { .. }     => "→v",
        SubnetNodeType::ConstFloat { .. }    => "→f",
        SubnetNodeType::ConstInt { .. }      => "→i",
        SubnetNodeType::ScatterPoints { .. } => "⁙",
        SubnetNodeType::GetTemplate          => "📄",
        SubnetNodeType::GetAttribute { .. }  => "🔍",
        SubnetNodeType::CopyToPoints         => "📦",
    }
}

#[derive(Clone, Debug)]
pub enum GetAttributeTarget {
    P,
    N,
    PtIndex,
    PrimIndex,
    PtsNumber,
    PrimsNumber,
    Uv,
    Custom(String),
}

// ============================================================================
// SUBNET SOCKET VALUE
// ============================================================================

#[derive(Clone, Debug)]
pub enum SubnetValue {
    Mesh(MeshData),
    Vec3(Vec3),
    Float(f32),
    Int(i32),
}

impl SubnetValue {
    pub fn as_mesh(&self)  -> Option<&MeshData> { if let Self::Mesh(m)  = self { Some(m) } else { None } }
    pub fn as_vec3(&self)  -> Option<Vec3>       { if let Self::Vec3(v)  = self { Some(*v) } else { None } }
    pub fn as_float(&self) -> Option<f32>        { if let Self::Float(f) = self { Some(*f) } else { None } }
    pub fn as_int(&self)   -> Option<i32>        { if let Self::Int(i)   = self { Some(*i) } else { None } }
}

// ============================================================================
// EVAL RESULT
// ============================================================================

#[derive(Clone, Debug)]
pub struct NamedMesh {
    pub path: String,
    pub mesh: MeshData,
}

#[derive(Clone, Debug)]
pub enum EvalResult {
    Single(MeshData),
    Named(Vec<NamedMesh>),
}

impl EvalResult {
    pub fn into_mesh(self) -> MeshData {
        match self {
            EvalResult::Single(m) => m,
            EvalResult::Named(prims) => prims.into_iter().map(|p| p.mesh).fold(
                MeshData::default(),
                |acc, m| crate::node_graph::nodes::merge(&acc, &m),
            ),
        }
    }

    pub fn as_mesh(&self) -> MeshData {
        self.clone().into_mesh()
    }
}

// ============================================================================
// PRIMITIVE INSPECTOR STATE
// ============================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub enum PrimInspectorTab {
    #[default]
    Vertex,
    Uniform,
    FaceVarying,
    Constant,
}

#[derive(Resource, Default)]
pub struct PrimInspectorState {
    pub active_tab:     PrimInspectorTab,
    pub row_offset:     usize,
    pub cached_mesh:    Option<MeshData>,
    pub cached_node_id: Option<NodeId>,
    pub graph_version:  u64,
}

impl PrimInspectorState {
    pub fn is_cache_valid(&self, node_id: Option<NodeId>, graph_version: u64) -> bool {
        self.cached_node_id == node_id && self.graph_version == graph_version
    }

    pub fn update_cache(&mut self, node_id: Option<NodeId>, mesh: MeshData, graph_version: u64) {
        self.cached_mesh    = Some(mesh);
        self.cached_node_id = node_id;
        self.graph_version  = graph_version;
    }

    pub fn clear_cache(&mut self, node_id: Option<NodeId>) {
        self.cached_node_id = node_id;
        self.cached_mesh    = None;
    }

    pub fn get_cached_mesh(&self) -> Option<&MeshData> {
        self.cached_mesh.as_ref()
    }

    pub fn set_active_tab(&mut self, tab: PrimInspectorTab) {
        if self.active_tab != tab {
            self.active_tab = tab;
            self.row_offset = 0;
        }
    }
}

// ============================================================================
// SCENE HIERARCHY
// ============================================================================

fn prim_icon(name: &str, has_children: bool) -> &'static str {
    let lower = name.to_lowercase();
    if has_children {
        if lower.contains("geom")                           { return "🔷"; }
        if lower.contains("xform")                          { return "⟲"; }
        if lower.contains("look") || lower.contains("mat") { return "🎨"; }
        return "📁";
    }
    if lower.contains("cylinder") || lower.contains("tube") { return "🔩"; }
    if lower.contains("sphere")   || lower.contains("ball") { return "🔵"; }
    if lower.contains("cube")     || lower.contains("box")  { return "🟫"; }
    if lower.contains("plane")    || lower.contains("grid") { return "⬜"; }
    if lower.contains("cone")                               { return "🔺"; }
    "🔹"
}

#[derive(Clone, Debug)]
pub struct SceneObject {
    pub id:           SceneObjectId,
    pub name:         String,
    pub icon:         &'static str,
    pub depth:        usize,
    pub has_children: bool,
    pub expanded:     bool,
    pub node_id:      NodeId,
    pub prim_path:    Option<String>,
}

#[derive(Resource, Default)]
pub struct SceneHierarchy {
    pub objects:            Vec<SceneObject>,
    pub selected:           Option<SceneObjectId>,
    pub selected_prim_path: Option<String>,
    next_id:                usize,
}

impl SceneHierarchy {
    fn next_id(&mut self) -> SceneObjectId {
        let id = SceneObjectId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn rebuild(&mut self, entries: Vec<(NodeId, String, EvalResult)>) {
        let expansions: HashMap<String, bool> = self
            .objects.iter()
            .map(|o| (o.name.clone(), o.expanded))
            .collect();

        self.objects.clear();

        for (node_id, node_name, result) in entries {
            match result {
                EvalResult::Single(_) => {
                    let id = self.next_id();
                    self.objects.push(SceneObject {
                        id,
                        name:         node_name.clone(),
                        icon:         "🔹",
                        depth:        0,
                        has_children: false,
                        expanded:     true,
                        node_id,
                        prim_path:    None,
                    });
                }

                EvalResult::Named(prims) => {
                    let expanded = *expansions.get(&node_name).unwrap_or(&true);
                    let group_id = self.next_id();
                    self.objects.push(SceneObject {
                        id:           group_id,
                        name:         node_name.clone(),
                        icon:         "📂",
                        depth:        0,
                        has_children: !prims.is_empty(),
                        expanded,
                        node_id,
                        prim_path:    None,
                    });

                    let mut seen_parents: std::collections::HashSet<String> =
                        std::collections::HashSet::new();

                    for prim in &prims {
                        let segs: Vec<&str> = prim.path
                            .trim_start_matches('/')
                            .split('/')
                            .collect();

                        for seg_depth in 1..segs.len().saturating_sub(1) {
                            let parent_key = segs[..=seg_depth].join("/");
                            if seen_parents.insert(parent_key.clone()) {
                                let exp = *expansions.get(segs[seg_depth]).unwrap_or(&true);
                                let id  = self.next_id();
                                self.objects.push(SceneObject {
                                    id,
                                    name:         segs[seg_depth].to_string(),
                                    icon:         prim_icon(segs[seg_depth], true),
                                    depth:        seg_depth,
                                    has_children: true,
                                    expanded:     exp,
                                    node_id,
                                    prim_path:    None,
                                });
                            }
                        }

                        let leaf_depth = segs.len().saturating_sub(1).max(1);
                        let leaf_name  = segs.last().unwrap_or(&"mesh").to_string();
                        let id         = self.next_id();
                        self.objects.push(SceneObject {
                            id,
                            name:         leaf_name.clone(),
                            icon:         prim_icon(&leaf_name, false),
                            depth:        leaf_depth,
                            has_children: false,
                            expanded:     true,
                            node_id,
                            prim_path:    Some(prim.path.clone()),
                        });
                    }
                }
            }
        }
    }
}