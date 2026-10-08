use bevy::prelude::*;
use bevy_egui::egui;

// ============================================================================
// BEVY COMPONENTS
// ============================================================================

#[derive(Component)]
pub struct MainCamera;

#[derive(Component)]
pub struct GeneratedMesh;

/// A generated mesh that follows the playhead: rebuilt when time moves,
/// while the others stay as they are.
#[derive(Component)]
pub struct PosedMesh;

#[derive(Component)]
pub struct GroundGrid;

/// Tracks the screen rect available for the 3D viewport (excluding egui panels).
#[derive(Resource, Default)]
pub struct ViewportRect(pub Option<egui::Rect>);

// ============================================================================
// SHARED IDs
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NodeId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SubnetId(pub usize);

// Scene object ID — one per visible object in the scene explorer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneObjectId(pub usize);

// ============================================================================
// OUTER NODE TYPES
// ============================================================================

/// Vec3 stored as a plain array in saved graphs.
mod vec3_array {
    use bevy::math::Vec3;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(v: &Vec3, s: S) -> Result<S::Ok, S::Error> {
        v.to_array().serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec3, D::Error> {
        <[f32; 3]>::deserialize(d).map(Vec3::from_array)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum NodeType {
    CreateCube   { size: f32 },
    CreateSphere { radius: f32, segments: u32 },
    CreateGrid   { rows: u32, cols: u32, size: f32 },
    LoadUsd      { path: String },
    /// Mark packed primitives whose path matches a pattern as picked.
    PickPrims    { pattern: String },
    /// Remove packed primitives whose path matches a pattern, or keep only those.
    PrunePrims   { pattern: String, keep: bool },
    /// Merge packed primitives into one mesh.
    UnpackPrims,
    /// Moves whatever comes in: a mesh, packed primitives (the picked ones,
    /// or every one), or a clip. A clip moves by its top joints, so a skinned
    /// mesh follows its skeleton once and is not moved a second time.
    /// `rotation` in radians, applied X then Y then Z.
    Transform    {
        #[serde(with = "vec3_array")] translation: Vec3,
        #[serde(with = "vec3_array")] rotation:    Vec3,
        #[serde(with = "vec3_array")] scale:       Vec3,
    },
    Merge,
    ScatterPoints { count: u32, seed: u32 },
    CopyToPoints,
    Subnet       { id: SubnetId, name: String },
    Output,

    // ── Animation ────────────────────────────────────────────────────────────
    // Each of these outputs a whole clip. Time is not a parameter: the
    // timeline samples the clip of the selected node.
    LoadFbx      { path: String, take: u32 },
    TestClip     { seconds: f32, fps_num: u32, fps_den: u32 },
    RenameJoints { find: String, replace: String, strip_namespace: bool, prefix: String },
    /// Frames removed from the head and the tail of the incoming clip.
    TrimClip     { head: u32, tail: u32 },
    Retime       { fps_num: u32, fps_den: u32, mode: RetimeMode },
    SetTimecode  { hours: u32, minutes: u32, seconds: u32, frames: u32, drop_frame: bool },

    // ── Batch / export ───────────────────────────────────────────────────────
    /// One FBX out of a folder, chosen by index into the sorted file list.
    LoadFbxDir   { dir: String, index: u32, take: u32 },
    /// One output per entry: the picked joint and everything below it.
    SplitSkeleton { picks: Vec<SplitPick> },
    /// Single-frame neutral pose. `hip_height` is in centimetres.
    AutoTPose    { set_hip_height: bool, hip_height: f32 },
    /// Manual corrections on top of a pose.
    FixPose      { edits: Vec<crate::core::anim::PoseEdit> },
    /// Spheres and cylinders bound to the skeleton.
    ProxySkin    { thickness: f32 },
    /// Passes the clip through. Writing happens from the properties panel.
    /// `mesh`: also write the skinned mesh, skin and bind pose. Off, the
    /// file holds the skeleton and its motion only, which is what an engine
    /// imports onto a skeleton it already has. Graphs saved before the
    /// switch existed wrote the mesh, and still do.
    WriteFbx     { path: String, #[serde(default = "yes")] mesh: bool },

    // ── Mocap tools ──────────────────────────────────────────────────────────
    /// Swap left and right.
    MirrorClip,
    /// Gaussian filter over time. `radius` in frames.
    SmoothClip   { radius: u32, amount: f32, translations: bool },
    /// Hold the hips over their starting point.
    InPlace      { keep_height: bool, to_root: bool },
    /// Graphs saved before Transform took clips. Read, then turned into a
    /// Transform node (`graph_io`).
    TransformClip { translate: [f32; 3], rotate: [f32; 3], scale: f32 },
    /// First input followed by the second, cross-faded over `blend` frames.
    BlendClips   { blend: u32, align: bool },
    /// Ease the end into the start so the clip cycles.
    LoopClip     { blend: u32 },
    /// Motion of the first input on the skeleton of the second.
    Retarget,
    TimeWarp     { speed: f32, reverse: bool },
    /// Remove joints whose name contains one of the comma-separated words.
    PruneJoints  { words: String },
    /// Put the lowest point of the clip at `height` (metres).
    FloorClip    { height: f32 },

    // ── Ragdoll ──────────────────────────────────────────────────────────────
    /// Every mesh of an FBX file, as packed primitives: a set to collide with.
    LoadFbxMesh  { path: String },
    /// Graphs saved before Body Collide showed its own bodies. Read, then
    /// folded into the Body Collide node before it (`graph_io`).
    Calamari     { hulls: bool, detail: u32 },
    /// Body Collide: keeps the character out of a collider mesh and out of
    /// itself. Solving is started from the properties panel. Saved under its
    /// first name, so older graphs open. `view` is what the viewport draws
    /// of the character, not what the node puts out.
    Ragdoll      { settings: crate::ragdoll::Settings, #[serde(default)] view: BodyView },

    // ── UV ───────────────────────────────────────────────────────────────────
    /// Make texture coordinates. `angle` (degrees) limits how far a chart's
    /// normals may spread; `margin` is the gap between charts.
    UvUnwrap     { method: crate::core::uv::UvMethod, angle: f32, margin: f32, axis: usize,
                   /// UDIM tiles to spread the elements over. One: everything in the unit square.
                   #[serde(default = "one_tile")] tiles: u32 },
    /// Move, turn and scale the whole UV layout.
    UvTransform  { offset: [f32; 2], rotate: f32, scale: [f32; 2] },
    /// Move, turn and scale single UV islands.
    UvEdit       { edits: Vec<crate::core::uv::IslandEdit> },

    // ── Modelling ────────────────────────────────────────────────────────────
    /// Polygon modelling in one node: an ordered list of operations, each
    /// with its own selection, like the history of an Edit Poly modifier.
    EditPoly {
        ops:     Vec<crate::core::poly::PolyOp>,
        /// Selection being built for the next operation.
        pending: crate::core::poly::PolySelection,
        /// Operation whose selection is being edited in the viewport, which
        /// then shows the mesh as it enters that operation. None: `pending`.
        edit:    Option<usize>,
        /// Collapse every earlier operation when a new one is added.
        #[serde(default)]
        auto_collapse: bool,
    },
}

/// What the viewport draws of a character that Body Collide works on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BodyView {
    /// Its skin, as the clip carries it.
    #[default]
    Skin,
    /// The skin cut into one rigid piece per body.
    Pieces,
    /// The convex hull of each body, as the solver collides it.
    Hulls,
}

/// What one output of the Split node keeps.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SplitPick {
    /// Nth character found in the clip. Survives name changes between files.
    Character(u32),
    /// Joint with this exact name.
    Joint(String),
}

pub const DEFAULT_WRITE_PATH: &str = "{dir}/split/{file}_{char}.fbx";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RetimeMode {
    /// Keep duration, interpolate new samples.
    Resample,
    /// Keep samples, relabel the rate (changes speed).
    Reinterpret,
}

impl NodeType {
    /// Nodes that create animation data with no input.
    pub fn is_anim_generator(&self) -> bool {
        matches!(self, NodeType::LoadFbx { .. } | NodeType::LoadFbxDir { .. } | NodeType::TestClip { .. })
    }

    /// Nodes a clip goes through: the clip nodes, and Transform.
    pub fn passes_clips(&self) -> bool { self.is_anim() || matches!(self, NodeType::Transform { .. }) }

    /// Nodes whose output is a clip.
    pub fn is_anim(&self) -> bool {
        self.is_anim_generator() || matches!(self,
            NodeType::RenameJoints { .. } | NodeType::TrimClip { .. } | NodeType::Retime { .. }
            | NodeType::SetTimecode { .. } | NodeType::SplitSkeleton { .. } | NodeType::AutoTPose { .. }
            | NodeType::FixPose { .. } | NodeType::ProxySkin { .. } | NodeType::WriteFbx { .. }
            | NodeType::MirrorClip | NodeType::SmoothClip { .. } | NodeType::InPlace { .. }
            | NodeType::TransformClip { .. } | NodeType::BlendClips { .. } | NodeType::LoopClip { .. }
            | NodeType::Retarget | NodeType::TimeWarp { .. } | NodeType::PruneJoints { .. }
            | NodeType::FloorClip { .. } | NodeType::Calamari { .. } | NodeType::Ragdoll { .. })
    }
}

pub fn node_type_icon(t: &NodeType) -> &'static str {
    match t {
        NodeType::CreateCube { .. }    => "◼",
        NodeType::CreateSphere { .. }  => "⚪",
        NodeType::CreateGrid { .. }    => "⊞",
        NodeType::LoadUsd { .. }       => "📂",
        NodeType::PickPrims { .. }     => "👆",
        NodeType::PrunePrims { .. }    => "🚫",
        NodeType::UnpackPrims          => "📦",
        NodeType::Transform { .. }     => "⟲",
        NodeType::Merge                => "➕",
        NodeType::ScatterPoints { .. } => "✳",
        NodeType::CopyToPoints         => "❇",
        NodeType::Subnet { .. }        => "▣",
        NodeType::Output               => "▶",
        NodeType::LoadFbx { .. }       => "🎬",
        NodeType::TestClip { .. }      => "🚶",
        NodeType::RenameJoints { .. }  => "✏",
        NodeType::TrimClip { .. }      => "✂",
        NodeType::Retime { .. }        => "⏱",
        NodeType::SetTimecode { .. }   => "🕐",
        NodeType::LoadFbxDir { .. }    => "📁",
        NodeType::SplitSkeleton { .. } => "Ψ",
        NodeType::AutoTPose { .. }     => "✚",
        NodeType::FixPose { .. }       => "🔧",
        NodeType::ProxySkin { .. }     => "⬟",
        NodeType::WriteFbx { .. }      => "💾",
        NodeType::EditPoly { .. }      => "🔨",
        NodeType::MirrorClip           => "↔",
        NodeType::SmoothClip { .. }    => "〰",
        NodeType::InPlace { .. }       => "📍",
        NodeType::TransformClip { .. } => "🔃",
        NodeType::BlendClips { .. }    => "🔀",
        NodeType::LoopClip { .. }      => "🔁",
        NodeType::Retarget             => "👥",
        NodeType::TimeWarp { .. }      => "⏩",
        NodeType::PruneJoints { .. }   => "🌿",
        NodeType::FloorClip { .. }     => "⬇",
        NodeType::LoadFbxMesh { .. }   => "📥",
        NodeType::Calamari { .. }      => "✂",
        NodeType::Ragdoll { .. }       => "💥",
        NodeType::UvUnwrap { .. }      => "🗺",
        NodeType::UvTransform { .. }   => "📌",
        NodeType::UvEdit { .. }        => "✋",
    }
}

pub fn node_type_label(t: &NodeType) -> &'static str {
    match t {
        NodeType::CreateCube { .. }    => "Cube",
        NodeType::CreateSphere { .. }  => "Sphere",
        NodeType::CreateGrid { .. }    => "Grid",
        NodeType::LoadUsd { .. }       => "Load USD",
        NodeType::PickPrims { .. }     => "Pick Primitives",
        NodeType::PrunePrims { .. }    => "Prune Primitives",
        NodeType::UnpackPrims          => "Unpack",
        NodeType::Transform { .. }     => "Transform",
        NodeType::Merge                => "Merge",
        NodeType::ScatterPoints { .. } => "Scatter Points",
        NodeType::CopyToPoints         => "Copy to Points",
        NodeType::Subnet { .. }        => "ICE",
        NodeType::Output               => "Output",
        NodeType::LoadFbx { .. }       => "Load FBX",
        NodeType::TestClip { .. }      => "Test Clip",
        NodeType::RenameJoints { .. }  => "Rename Joints",
        NodeType::TrimClip { .. }      => "Trim",
        NodeType::Retime { .. }        => "Retime",
        NodeType::SetTimecode { .. }   => "Set Timecode",
        NodeType::LoadFbxDir { .. }    => "Load FBX Folder",
        NodeType::SplitSkeleton { .. } => "Split Characters",
        NodeType::AutoTPose { .. }     => "Auto T-Pose",
        NodeType::FixPose { .. }       => "Fix Pose",
        NodeType::ProxySkin { .. }     => "Proxy Skin",
        NodeType::WriteFbx { .. }      => "Write FBX",
        NodeType::EditPoly { .. }      => "Edit Poly",
        NodeType::MirrorClip           => "Mirror",
        NodeType::SmoothClip { .. }    => "Smooth",
        NodeType::InPlace { .. }       => "In Place",
        NodeType::TransformClip { .. } => "Transform Clip",
        NodeType::BlendClips { .. }    => "Blend",
        NodeType::LoopClip { .. }      => "Loop",
        NodeType::Retarget             => "Retarget",
        NodeType::TimeWarp { .. }      => "Time Warp",
        NodeType::PruneJoints { .. }   => "Prune Joints",
        NodeType::FloorClip { .. }     => "Floor",
        NodeType::LoadFbxMesh { .. }   => "Load FBX Mesh",
        NodeType::Calamari { .. }      => "Calamari",
        NodeType::Ragdoll { .. }       => "Body Collide",
        NodeType::UvUnwrap { .. }      => "UV Unwrap",
        NodeType::UvTransform { .. }   => "UV Transform",
        NodeType::UvEdit { .. }        => "UV Edit",
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

pub fn subnet_node_label(t: &SubnetNodeType) -> &'static str {
    match t {
        SubnetNodeType::SubInput            => "Subnet Input",
        SubnetNodeType::SubOutput           => "Subnet Output",
        SubnetNodeType::AddVec3             => "Add",
        SubnetNodeType::SubtractVec3        => "Subtract",
        SubnetNodeType::MultiplyVec3 { .. } => "Multiply",
        SubnetNodeType::CrossProduct        => "Cross Product",
        SubnetNodeType::Normalize           => "Normalize",
        SubnetNodeType::DotProduct          => "Dot Product",
        SubnetNodeType::LerpVec3 { .. }     => "Blend",
        SubnetNodeType::ConstVec3 { .. }    => "Vector",
        SubnetNodeType::ConstFloat { .. }   => "Number",
        SubnetNodeType::ConstInt { .. }     => "Integer",
        SubnetNodeType::ScatterPoints { .. } => "Scatter Points",
        SubnetNodeType::GetTemplate          => "Get Template",
        SubnetNodeType::CopyToPoints         => "Copy to Points",
    }
}

pub fn subnet_node_icon(t: &SubnetNodeType) -> &'static str {
    match t {
        SubnetNodeType::SubInput            => "▶",
        SubnetNodeType::SubOutput           => "◀",
        SubnetNodeType::AddVec3             => "+",
        SubnetNodeType::SubtractVec3        => "-",
        SubnetNodeType::MultiplyVec3 { .. } => "×",
        SubnetNodeType::CrossProduct        => "×",
        SubnetNodeType::Normalize           => "|v|",
        SubnetNodeType::DotProduct          => "·",
        SubnetNodeType::LerpVec3 { .. }     => "≈",
        SubnetNodeType::ConstVec3 { .. }    => "→v",
        SubnetNodeType::ConstFloat { .. }   => "→f",
        SubnetNodeType::ConstInt { .. }     => "→i",
        SubnetNodeType::ScatterPoints { .. } => "✳",
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
    /// Polygons as vertex loops, when the mesh has them. `indices` then holds
    /// their triangulation. Empty for meshes that are only triangles.
    pub polys:      Vec<Vec<u32>>,
    /// Texture coordinates, one pair per triangle corner in the order of
    /// `indices`. Empty when the mesh has none.
    pub uvs:        Vec<[f32; 2]>,
}

impl MeshData {
    pub fn from_triangles(vertices: Vec<[f32; 3]>, indices: Vec<u32>) -> Self {
        let face_count = indices.len() / 3;
        Self { vertices, indices, face_count, ..Default::default() }
    }

    /// Mesh from polygons of any size. Triangulates them for drawing and
    /// keeps the polygons for modelling.
    pub fn from_polys(vertices: Vec<[f32; 3]>, polys: Vec<Vec<u32>>) -> Self {
        let mut indices = Vec::new();
        for poly in &polys {
            for i in 1..poly.len().saturating_sub(1) {
                indices.extend([poly[0], poly[i], poly[i + 1]]);
            }
        }
        let mut m = Self { vertices, indices, face_count: polys.len(), polys, ..Default::default() };
        m.compute_normals();
        m
    }

    /// The mesh's polygons: the stored ones, or its triangles.
    pub fn polygons(&self) -> Vec<Vec<u32>> {
        if !self.polys.is_empty() { return self.polys.clone(); }
        self.indices.chunks_exact(3).map(|t| t.to_vec()).collect()
    }

    /// Positions, normals and triangle indices for drawing. A mesh with
    /// polygons is drawn with hard edges where neighbouring polygons meet at
    /// more than 40 degrees and smooth shading elsewhere, so a box looks like
    /// a box and a sphere like a sphere.
    pub fn render_buffers(&self) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>) {
        use bevy::math::Vec3;
        if self.polys.is_empty() {
            let normals = if self.normals.len() == self.vertices.len() {
                self.normals.clone()
            } else {
                self.vertices.iter().map(|_| [0.0f32, 1.0, 0.0]).collect()
            };
            return (self.vertices.clone(), normals, self.indices.clone());
        }
        let v = |i: u32| Vec3::from_array(self.vertices[i as usize]);
        let face_n: Vec<Vec3> = self.polys.iter().map(|poly| {
            let mut n = Vec3::ZERO;
            for i in 0..poly.len() { n += v(poly[i]).cross(v(poly[(i + 1) % poly.len()])); }
            n
        }).collect();
        let mut around: Vec<Vec<u32>> = vec![vec![]; self.vertices.len()];
        for (p, poly) in self.polys.iter().enumerate() {
            for i in poly { around[*i as usize].push(p as u32); }
        }
        let limit = 40f32.to_radians().cos();
        let (mut pos, mut nrm, mut idx) = (vec![], vec![], vec![]);
        for (p, poly) in self.polys.iter().enumerate() {
            let own  = face_n[p].normalize_or_zero();
            let base = pos.len() as u32;
            for i in poly {
                // Area-weighted average over the neighbours within the angle limit.
                let n: Vec3 = around[*i as usize].iter()
                    .map(|q| face_n[*q as usize])
                    .filter(|q| q.normalize_or_zero().dot(own) >= limit)
                    .sum();
                pos.push(self.vertices[*i as usize]);
                nrm.push(if n.length_squared() > 0.0 { n.normalize().to_array() } else { own.to_array() });
            }
            for i in 1..poly.len().saturating_sub(1) {
                idx.extend([base, base + i as u32, base + i as u32 + 1]);
            }
        }
        (pos, nrm, idx)
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
            // A vertex used only by triangles with no area has no direction of its own.
            if c > 0 && n.length_squared() > 0.0 { n.normalize().to_array() } else { [0.0, 1.0, 0.0] }
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
    pub path:     String,
    /// Shared: a packed primitive is passed along without being copied.
    pub mesh:     std::sync::Arc<MeshData>,
    /// Chosen by a Pick Primitives node. Nodes that change a mesh then work
    /// on the picked primitives and pass the others through untouched.
    pub picked:   bool,
    /// Path of the material bound to it in the file it came from.
    pub material: Option<String>,
    /// That material, as far as the viewport can show it.
    pub look:     Option<std::sync::Arc<Look>>,
}

/// How a surface looks in the viewport.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub name:      String,
    pub color:     [f32; 3],
    pub roughness: f32,
    pub metallic:  f32,
    pub opacity:   f32,
    /// Texture files, as full paths.
    pub color_map:    Option<std::path::PathBuf>,
    pub emissive_map: Option<std::path::PathBuf>,
    /// The colour texture's alpha cuts the surface out.
    pub cutout:    bool,
}

impl NamedMesh {
    pub fn new(path: String, mesh: MeshData) -> Self {
        Self { path, mesh: std::sync::Arc::new(mesh), picked: false, material: None, look: None }
    }
}

#[derive(Clone, Debug)]
pub enum EvalResult {
    Single(MeshData),
    Named(Vec<NamedMesh>),
    /// Skeleton + clip. Shared, so passing it along the graph is cheap.
    Anim(std::sync::Arc<crate::core::anim::AnimData>),
}

impl EvalResult {
    pub fn into_mesh(self) -> MeshData {
        match self {
            EvalResult::Single(m) => m,
            EvalResult::Named(prims) => {
                let all: Vec<&MeshData> = prims.iter().map(|p| &*p.mesh).collect();
                crate::node_graph::nodes::merge_all(&all)
            }
            EvalResult::Anim(_) => MeshData::default(),
        }
    }

    pub fn as_anim(&self) -> Option<&std::sync::Arc<crate::core::anim::AnimData>> {
        if let EvalResult::Anim(a) = self { Some(a) } else { None }
    }

    pub fn as_mesh(&self) -> MeshData {
        self.clone().into_mesh()
    }

    /// Everything as one mesh, shared. A single packed primitive is handed
    /// over as it is. Several are merged once and the merge is kept while
    /// the same primitives keep arriving, so a heavy model is not copied
    /// each time the graph is evaluated.
    pub fn shared_mesh(&self) -> std::sync::Arc<MeshData> {
        use std::sync::{Arc, Mutex};
        type Kept = (Vec<Arc<MeshData>>, Arc<MeshData>);
        static MERGED: Mutex<Vec<Kept>> = Mutex::new(Vec::new());
        match self {
            EvalResult::Named(prims) if prims.len() == 1 => prims[0].mesh.clone(),
            EvalResult::Named(prims) => {
                let mut kept = MERGED.lock().unwrap();
                if let Some((_, m)) = kept.iter().find(|(parts, _)| parts.len() == prims.len() && parts.iter().zip(prims).all(|(a, b)| Arc::ptr_eq(a, &b.mesh))) {
                    return m.clone();
                }
                let all: Vec<&MeshData> = prims.iter().map(|p| &*p.mesh).collect();
                let merged = Arc::new(crate::node_graph::nodes::merge_all(&all));
                if kept.len() >= 2 { kept.remove(0); }
                kept.push((prims.iter().map(|p| p.mesh.clone()).collect(), merged.clone()));
                merged
            }
            other => Arc::new(other.as_mesh()),
        }
    }

    /// True when this is packed primitives with at least one picked.
    pub fn has_picked(&self) -> bool {
        matches!(self, EvalResult::Named(prims) if prims.iter().any(|p| p.picked))
    }

    /// The mesh a modifying node works on: the picked primitives as one
    /// mesh when some are picked, everything as one mesh otherwise.
    pub fn work_mesh(&self) -> MeshData {
        match self {
            EvalResult::Named(prims) if prims.iter().any(|p| p.picked) => {
                let picked: Vec<&MeshData> = prims.iter().filter(|p| p.picked).map(|p| &*p.mesh).collect();
                crate::node_graph::nodes::merge_all(&picked)
            }
            other => other.as_mesh(),
        }
    }

    /// Run a mesh operation. On packed primitives with some picked, only
    /// those go through it: they come out as one primitive, still picked,
    /// in the place of the first of them, and the rest pass through as they
    /// are. Otherwise everything is merged and goes through.
    pub fn map_mesh(&self, op: impl FnOnce(MeshData) -> MeshData) -> EvalResult {
        match self {
            EvalResult::Named(prims) if prims.iter().any(|p| p.picked) => {
                let result = op(self.work_mesh());
                let mut out: Vec<NamedMesh> = Vec::with_capacity(prims.len());
                let mut result = Some(result);
                for p in prims {
                    if !p.picked { out.push(p.clone()); continue; }
                    if let Some(mesh) = result.take() {
                        out.push(NamedMesh { path: p.path.clone(), mesh: std::sync::Arc::new(mesh), picked: true, material: p.material.clone(), look: p.look.clone() });
                    }
                }
                EvalResult::Named(out)
            }
            other => EvalResult::Single(op(other.as_mesh())),
        }
    }

    /// Paths of the packed primitives, in order.
    pub fn prim_paths(&self) -> Vec<String> {
        match self { EvalResult::Named(prims) => prims.iter().map(|p| p.path.clone()).collect(), _ => vec![] }
    }
}

// ============================================================================
// PRIMITIVE INSPECTOR STATE  (Bevy Resource)
// ============================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub enum PrimInspectorTab {
    #[default]
    Vertex,
    Edge,
    /// One row per polygon. Per-polygon primvars sit here too.
    Uniform,
    FaceVarying,
    Constant,
    /// Joints of a clip, at the playhead.
    Joint,
    /// Bones of a clip: each runs from one joint to another.
    Bone,
}

/// One tab of the inspector, ready to draw.
#[derive(Clone, Debug, Default)]
pub struct InspectorTable {
    pub rows:   usize,
    /// Text column before the numbers (joint names), with its heading.
    pub labels: Option<(String, Vec<String>)>,
    /// Column group name and its values per row. A group wider than one
    /// is shown as name.X, name.Y, name.Z.
    pub cols:   Vec<(String, Vec<Vec<f32>>)>,
}

#[derive(Resource, Default)]
pub struct PrimInspectorState {
    pub active_tab:   PrimInspectorTab,
    pub row_offset:   usize,

    // Mesh or clip of the selected node, cooked once per graph revision.
    cached_mesh:    Option<std::sync::Arc<MeshData>>,
    cached_clip:    Option<std::sync::Arc<crate::core::anim::AnimData>>,
    cached_node_id: Option<NodeId>,
    cached_rev:     Option<u64>,
    /// Number of edges of the cached mesh.
    pub edge_count: usize,
    // The open tab, built once per cached data (and per frame for a clip).
    table_key: Option<(PrimInspectorTab, usize)>,
    table:     InspectorTable,
}

impl PrimInspectorState {
    pub fn is_cache_valid(&self, node_id: Option<NodeId>, revision: u64) -> bool {
        self.cached_node_id == node_id && self.cached_rev == Some(revision)
    }

    pub fn update_cache(
        &mut self,
        node_id:  Option<NodeId>,
        mesh:     Option<MeshData>,
        clip:     Option<std::sync::Arc<crate::core::anim::AnimData>>,
        revision: u64,
    ) {
        self.edge_count = mesh.as_ref()
            .map(|m| crate::core::poly::PolyMesh::from_mesh(m).edges().len()).unwrap_or(0);
        self.cached_mesh    = mesh.map(std::sync::Arc::new);
        self.cached_clip    = clip;
        self.cached_node_id = node_id;
        self.cached_rev     = Some(revision);
        self.table_key      = None;
        self.table          = InspectorTable::default();
    }

    pub fn cached_mesh(&self) -> Option<std::sync::Arc<MeshData>> { self.cached_mesh.clone() }
    pub fn cached_clip(&self) -> Option<std::sync::Arc<crate::core::anim::AnimData>> { self.cached_clip.clone() }

    /// True when the stored table is this tab at this frame.
    pub fn table_is_for(&self, tab: &PrimInspectorTab, frame: usize) -> bool {
        self.table_key.as_ref() == Some(&(tab.clone(), frame))
    }

    pub fn set_table(&mut self, tab: PrimInspectorTab, frame: usize, table: InspectorTable) {
        self.table_key = Some((tab, frame));
        self.table     = table;
    }

    /// Lend the table out for drawing; hand it back with `put_table`.
    pub fn take_table(&mut self) -> InspectorTable { std::mem::take(&mut self.table) }
    pub fn put_table(&mut self, table: InspectorTable) { self.table = table; }

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
    if lower.contains("cube")     || lower.contains("box")  { return "⬛"; }
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

                EvalResult::Anim(anim) => {
                    // Skeleton: one row per joint, indented by hierarchy depth.
                    let expanded = *expansions.get(&node_name).unwrap_or(&true);
                    let id = self.next_id();
                    self.objects.push(SceneObject {
                        id,
                        name:         node_name.clone(),
                        icon:         "🎬",
                        depth:        0,
                        has_children: !anim.joints.is_empty(),
                        expanded,
                        node_id,
                        prim_path:    None,
                    });
                    let depths = anim.joint_depths();
                    let mut is_parent = vec![false; anim.joints.len()];
                    for j in &anim.joints {
                        if let Some(p) = j.parent { is_parent[p] = true; }
                    }
                    // Depth-first order so children sit under their parent.
                    let mut kids: Vec<Vec<usize>> = vec![vec![]; anim.joints.len()];
                    let mut todo: Vec<usize> = vec![];
                    for (i, j) in anim.joints.iter().enumerate().rev() {
                        match j.parent { Some(p) => kids[p].push(i), None => todo.push(i) }
                    }
                    while let Some(i) = todo.pop() {
                        let name = anim.joints[i].name.clone();
                        let exp  = *expansions.get(&name).unwrap_or(&true);
                        let id   = self.next_id();
                        self.objects.push(SceneObject {
                            id,
                            name,
                            icon:         if is_parent[i] { "●" } else { "○" },
                            depth:        depths[i] + 1,
                            has_children: is_parent[i],
                            expanded:     exp,
                            node_id,
                            prim_path:    None,
                        });
                        todo.extend(kids[i].iter().copied());
                    }
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
fn yes() -> bool { true }

fn one_tile() -> u32 { 1 }
