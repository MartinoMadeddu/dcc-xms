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

/// Tracks the screen rect available for the 3D viewport (excluding egui panels).
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

// Scene object ID — one per visible object in the scene explorer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneObjectId(pub usize);

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
    LerpVec3 { t: f32 },
    ConstVec3  { value: Vec3 },
    ConstFloat { value: f32  },
    ConstInt   { value: i32  },

    ScatterPoints { count: u32, seed: u32 },
    GetTemplate,
    CopyToPoints,
}

pub fn subnet_node_icon(t: &SubnetNodeType) -> &'static str {
    match t {
        SubnetNodeType::SubInput            => "▶",
        SubnetNodeType::SubOutput           => "◀",
        SubnetNodeType::AddVec3             => "＋",
        SubnetNodeType::SubtractVec3        => "－",
        SubnetNodeType::MultiplyVec3 { .. } => "✕",
        SubnetNodeType::CrossProduct        => "×",
        SubnetNodeType::Normalize           => "|v|",
        SubnetNodeType::DotProduct          => "·",
        SubnetNodeType::LerpVec3 { .. }     => "≈",
        SubnetNodeType::ConstVec3 { .. }    => "→v",
        SubnetNodeType::ConstFloat { .. }   => "→f",
        SubnetNodeType::ConstInt { .. }     => "→i",
        SubnetNodeType::ScatterPoints { .. } => "⁙",
        SubnetNodeType::GetTemplate          => "📄",
        SubnetNodeType::CopyToPoints         => "📦",
    }
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
    pub fn as_mesh(&self)  -> Option<&MeshData> { if let SubnetValue::Mesh(m)  = self { Some(m)  } else { None } }
    pub fn as_vec3(&self)  -> Option<Vec3>       { if let SubnetValue::Vec3(v)  = self { Some(*v) } else { None } }
    pub fn as_float(&self) -> Option<f32>        { if let SubnetValue::Float(f) = self { Some(*f) } else { None } }
    pub fn as_int(&self)   -> Option<i32>        { if let SubnetValue::Int(i)   = self { Some(*i) } else { None } }
}

// ============================================================================
// MESH DATA
// ============================================================================

// A single named primvar channel. The outer Vec is per-element
/// (vertex, face, or facevarying), the inner Vec is the tuple width (1, 2, 3, 4).
#[derive(Clone, Debug, Default)]
pub struct PrimVar {
    pub name:        String,
    pub interp:      PrimVarInterp,
    pub values:      Vec<Vec<f32>>,   // values[elem_idx][component]
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum PrimVarInterp {
    #[default]
    Vertex,       // one value per vertex/point
    Uniform,      // one value per face
    FaceVarying,  // one value per face-vertex
    Constant,     // one value for the whole prim
}

#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub vertices:   Vec<[f32; 3]>,
    pub indices:    Vec<u32>,
    pub points:     Vec<[f32; 3]>,   // scatter / point-cloud points
    pub normals:    Vec<[f32; 3]>,   // per-vertex normals (may be empty)
    pub primvars:   Vec<PrimVar>,    // arbitrary extra channels
    /// Original face-vertex counts before triangulation (for face count display)
    pub face_count: usize,
}

impl MeshData {
    pub fn from_triangles(vertices: Vec<[f32; 3]>, indices: Vec<u32>) -> Self {
        let face_count = indices.len() / 3;
        Self { vertices, indices, face_count, ..Default::default() }
    }

    /// Compute flat (per-triangle) normals and store them as a Vertex primvar.
    /// Called by generators after building geometry.
    pub fn compute_normals(&mut self) {
        use bevy::math::Vec3;
        let n = self.vertices.len();
        let mut normals = vec![Vec3::ZERO; n];
        let mut counts  = vec![0u32; n];

        for tri in self.indices.chunks(3) {
            if tri.len() < 3 { continue; }
            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            if a >= n || b >= n || c >= n { continue; }
            let va = Vec3::from(self.vertices[a]);
            let vb = Vec3::from(self.vertices[b]);
            let vc = Vec3::from(self.vertices[c]);
            let face_n = (vb - va).cross(vc - va);
            normals[a] += face_n; counts[a] += 1;
            normals[b] += face_n; counts[b] += 1;
            normals[c] += face_n; counts[c] += 1;
        }

        self.normals = normals.iter().zip(&counts).map(|(n, &c)| {
            if c > 0 { n.normalize().to_array() } else { [0.0, 1.0, 0.0] }
        }).collect();

        // Also store as a primvar so the inspector can show it
        self.primvars.retain(|p| p.name != "N");
        self.primvars.push(PrimVar {
            name:   "N".into(),
            interp: PrimVarInterp::Vertex,
            values: self.normals.iter()
                .map(|n| vec![n[0], n[1], n[2]])
                .collect(),
        });
    }

    /// Total number of faces (triangles after triangulation).
    pub fn num_faces(&self) -> usize { self.face_count }

    /// Total number of face-varying elements (3 per triangle).
    pub fn num_face_varying(&self) -> usize { self.indices.len() }
}

// ============================================================================
// EVAL RESULT
// ============================================================================

#[derive(Clone, Debug)]
pub struct NamedMesh {
    pub path:  String,
    pub mesh:  MeshData,
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
// PRIMITIVE INSPECTOR STATE  (Bevy Resource)
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
    pub active_tab:   PrimInspectorTab,
    pub row_offset:   usize,
    
    // ✅ NEW - Caching system
    cached_mesh: Option<MeshData>,
    cached_node_id: Option<NodeId>,
    graph_version: u64,  // Increments when graph changes
}

impl PrimInspectorState {
    /// Check if cache is valid for this node
    pub fn is_cache_valid(&self, node_id: Option<NodeId>, graph_version: u64) -> bool {
        self.cached_node_id == node_id && self.graph_version == graph_version
    }

    /// Update cache
    pub fn update_cache(&mut self, node_id: Option<NodeId>, mesh: MeshData, graph_version: u64) {
        self.cached_mesh = Some(mesh);
        self.cached_node_id = node_id;
        self.graph_version = graph_version;
    }

    /// Get cached mesh if valid
    pub fn get_cached_mesh(&self) -> Option<&MeshData> {
        self.cached_mesh.as_ref()
    }

    /// Clear cache when tab changes
    pub fn set_active_tab(&mut self, tab: PrimInspectorTab) {
        if self.active_tab != tab {
            self.active_tab = tab;
            self.row_offset = 0;
        }
    }
}

// ============================================================================
// SCENE HIERARCHY  (Bevy Resource)
// ============================================================================

/// Pick a display icon based on the prim name and whether it has children.
/// Matches common USD naming conventions (Geom, pCylinder, pSphere, etc.).
fn prim_icon(name: &str, has_children: bool) -> &'static str {
    let lower = name.to_lowercase();
    if has_children {
        if lower.contains("geom")                            { return "🔷"; }
        if lower.contains("xform")                            { return "⟲"; }
        if lower.contains("look") || lower.contains("mat")  { return "🎨"; }
        return "📁";  // generic xform / group
    }
    // Leaf mesh — guess from name
    if lower.contains("cylinder") || lower.contains("tube") { return "🔩"; }
    if lower.contains("sphere")   || lower.contains("ball") { return "🔵"; }
    if lower.contains("cube")     || lower.contains("box")  { return "🟫"; }
    if lower.contains("plane")    || lower.contains("grid") { return "⬜"; }
    if lower.contains("cone")                               { return "🔺"; }
    "🔹"  // generic mesh leaf
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
    /// Full USD prim path for leaf mesh objects (e.g. "/root/Chair/Geom/pCylinder1").
    /// None for group/xform nodes and for procedural Single meshes.
    pub prim_path:    Option<String>,
}

#[derive(Resource, Default)]
pub struct SceneHierarchy {
    pub objects:            Vec<SceneObject>,
    pub selected:           Option<SceneObjectId>,
    /// The prim path of the currently selected leaf, if any.
    /// Used by the Primitive Inspector to show only that prim's data.
    pub selected_prim_path: Option<String>,
    next_id:                usize,
}

impl SceneHierarchy {
    fn next_id(&mut self) -> SceneObjectId {
        let id = SceneObjectId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Rebuild from the evaluated node graph results.
    /// Expansion state is preserved by stable path key so toggling survives
    /// graph changes. Children are ALWAYS emitted — the UI handles hiding them.
pub fn rebuild(&mut self, entries: Vec<(NodeId, String, EvalResult)>) {
        let expansions: std::collections::HashMap<String, bool> = self
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

                        // Intermediate Xform parent rows — no prim_path
                        for seg_depth in 1..segs.len().saturating_sub(1) {
                            let parent_key = segs[..=seg_depth].join("/");
                            if seen_parents.insert(parent_key.clone()) {
                                let exp = *expansions.get(segs[seg_depth]).unwrap_or(&true);
                                let id = self.next_id();
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

                        // Leaf mesh entry — store the full prim path
                        let leaf_depth = segs.len().saturating_sub(1).max(1);
                        let leaf_name  = segs.last().unwrap_or(&"mesh").to_string();
                        let id = self.next_id();
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