//! Prim schemas: the data each kind of prim carries, shaped after UsdGeom,
//! UsdShade, UsdLux and UsdRender so that USD (or another DCC) maps onto it
//! directly and edits can be written back as USD attributes.

use crate::math::*;
use crate::path::Path;
use crate::time::{Lerp, Sampled};

/// Primvar interpolation (UsdGeom).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Constant,
    Uniform,
    Varying,
    Vertex,
    FaceVarying,
}

/// Primvar values; each variant interpolates over time element-wise.
#[derive(Clone, Debug, PartialEq)]
pub enum PrimvarValues {
    Float(Vec<f32>),
    Float2(Vec<Vec2f>),
    Float3(Vec<Vec3f>),
    Float4(Vec<Vec4f>),
    Int(Vec<i32>),
}

impl PrimvarValues {
    pub fn len(&self) -> usize {
        match self {
            PrimvarValues::Float(v) => v.len(),
            PrimvarValues::Float2(v) => v.len(),
            PrimvarValues::Float3(v) => v.len(),
            PrimvarValues::Float4(v) => v.len(),
            PrimvarValues::Int(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Lerp for PrimvarValues {
    fn lerp(&self, b: &Self, t: f64) -> Self {
        use PrimvarValues::*;
        match (self, b) {
            (Float(x), Float(y)) => Float(x.lerp(y, t)),
            (Float2(x), Float2(y)) => Float2(x.lerp(y, t)),
            (Float3(x), Float3(y)) => Float3(x.lerp(y, t)),
            (Float4(x), Float4(y)) => Float4(x.lerp(y, t)),
            // Integers and mismatched types hold
            _ => {
                if t < 1.0 {
                    self.clone()
                } else {
                    b.clone()
                }
            }
        }
    }
}

/// A named primvar (`primvars:<name>`), with optional `:indices`.
#[derive(Clone, Debug, PartialEq)]
pub struct Primvar {
    pub name: String,
    /// Authored interpolation; `None` = unauthored (consumers apply USD's default,
    /// or infer it from the array length as the importer does)
    pub interpolation: Option<Interpolation>,
    pub values: Sampled<PrimvarValues>,
    pub indices: Option<Vec<u32>>,
}

impl Subdivision {
    /// An authored subdivision scheme renderers refine (`catmullClark` or `loop`).
    /// Only authored schemes count: exporters often leave plain polygon meshes
    /// unauthored, and USD's schema fallback would make them all subdivision surfaces.
    pub fn is_tagged(&self) -> bool {
        matches!(self.scheme.as_deref(), Some("catmullClark" | "loop"))
    }
}

/// Subdivision settings (UsdGeomMesh `subdivisionScheme` and tags).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Subdivision {
    /// Authored `subdivisionScheme` (`catmullClark`, `loop`, `bilinear`, `none`);
    /// `None` = unauthored
    pub scheme: Option<String>,
    /// Refinement level requested for this mesh (renderer attribute), if any
    pub level: Option<u32>,
    /// `interpolateBoundary`: "none", "edgeOnly" or "edgeAndCorner"
    pub interpolate_boundary: Option<String>,
    pub crease_indices: Vec<u32>,
    pub crease_lengths: Vec<u32>,
    pub crease_sharpnesses: Vec<f32>,
    pub corner_indices: Vec<u32>,
    pub corner_sharpnesses: Vec<f32>,
}

/// A GeomSubset of faces with its own material binding.
#[derive(Clone, Debug, PartialEq)]
pub struct GeomSubset {
    pub name: String,
    pub faces: Vec<u32>,
    pub material_binding: Option<Path>,
}

/// UsdGeomMesh. Topology is constant; points and primvars may be time-sampled.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Mesh {
    pub points: Sampled<Vec<Vec3f>>,
    pub face_vertex_counts: Vec<u32>,
    pub face_vertex_indices: Vec<u32>,
    pub hole_indices: Vec<u32>,
    pub left_handed: bool,
    pub subdivision: Subdivision,
    /// `normals` (or `primvars:normals`)
    pub normals: Option<Primvar>,
    /// Every other primvar, texture coordinates included (`st`, …)
    pub primvars: Vec<Primvar>,
    pub subsets: Vec<GeomSubset>,
}

/// UsdGeomBasisCurves.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Curves {
    /// "linear" or "cubic"
    pub curve_type: String,
    /// "bezier", "bspline", "catmullRom" (cubic only)
    pub basis: String,
    /// "nonperiodic", "periodic", "pinned"
    pub wrap: String,
    pub curve_vertex_counts: Vec<u32>,
    pub points: Sampled<Vec<Vec3f>>,
    pub widths: Option<Primvar>,
    pub normals: Option<Primvar>,
    pub primvars: Vec<Primvar>,
}

/// Axis of a cylinder, cone, capsule or plane (UsdGeom `axis`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Axis {
    X,
    Y,
    #[default]
    Z,
}

/// UsdGeom implicit shapes (gprims), kept implicit: renderers may draw them
/// analytically (spheres) or tessellate them at sync.
#[derive(Clone, Debug, PartialEq)]
pub enum Gprim {
    Sphere { radius: f64 },
    Cube { size: f64 },
    Cylinder { radius: f64, height: f64, axis: Axis },
    Cone { radius: f64, height: f64, axis: Axis },
    Capsule { radius: f64, height: f64, axis: Axis },
    Plane { width: f64, length: f64, axis: Axis },
}

/// UsdGeomPoints.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Points {
    pub points: Sampled<Vec<Vec3f>>,
    pub widths: Option<Primvar>,
    pub primvars: Vec<Primvar>,
}

/// UsdGeomPointInstancer.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Instancer {
    /// `prototypes` relationship targets
    pub prototypes: Vec<Path>,
    pub proto_indices: Vec<u32>,
    pub positions: Sampled<Vec<Vec3f>>,
    /// Quaternions (i, j, k, real); interpolated with normalized lerp
    pub orientations: Option<Sampled<Vec<Quatf>>>,
    pub scales: Option<Sampled<Vec<Vec3f>>>,
    pub invisible_ids: Vec<i64>,
    pub ids: Option<Vec<i64>>,
}

/// One shader node of a material network (UsdShade / MaterialX).
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderNode {
    pub path: Path,
    /// `info:id` (e.g. `ND_open_pbr_surface_surfaceshader`, `UsdPreviewSurface`)
    pub id: String,
    /// The prim's USD type (`Shader`, …), for nodes without an `info:id`
    pub type_name: String,
    pub inputs: Vec<ShaderInput>,
}

/// A shader input: a constant value, or a connection to another node's output.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderInput {
    pub name: String,
    pub value: ShaderValue,
    /// `colorSpace` metadata authored on the input (texture `file` inputs)
    pub color_space: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShaderValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    Vec2(Vec2f),
    Vec3(Vec3f),
    Vec4(Vec4f),
    String(String),
    /// A file path (`asset`)
    Asset(String),
    /// Connected to `node`'s output `output`
    Connection { node: Path, output: String },
}

/// UsdShadeMaterial: its shader network and terminal connections. Compiled by
/// the renderer's sync, so edits stay on USD-level values.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Material {
    pub nodes: Vec<ShaderNode>,
    /// Surface terminal (`outputs:surface` / `outputs:mtlx:surface`): node path and output
    pub surface: Option<(Path, String)>,
    pub displacement: Option<(Path, String)>,
    pub volume: Option<(Path, String)>,
    /// A MaterialX document this material comes from, if any (path, target material)
    pub mtlx_document: Option<(String, String)>,
}

/// UsdLux light kinds, with their shape parameters.
#[derive(Clone, Debug, PartialEq)]
pub enum LightKind {
    Distant { angle: f32 },
    Sphere { radius: f32, treat_as_point: bool },
    Rect { width: f32, height: f32, texture: Option<String> },
    Disk { radius: f32 },
    Cylinder { radius: f32, length: f32, treat_as_line: bool },
    /// `pole_axis`: `DomeLight_1`'s `poleAxis` token ("Y", "Z" or "scene")
    Dome { texture: Option<String>, format: Option<String>, pole_axis: Option<String> },
}

/// UsdLux light. Intensity-like values may be animated.
#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    pub intensity: Sampled<f32>,
    pub exposure: Sampled<f32>,
    pub color: Sampled<Vec3f>,
    pub normalize: bool,
    pub enable_color_temperature: bool,
    pub color_temperature: f32,
    /// `shaping:cone:angle` / `shaping:cone:softness` / `shaping:focus`
    pub cone_angle: Option<f32>,
    pub cone_softness: Option<f32>,
    pub focus: Option<f32>,
    pub camera_visible: bool,
}

/// UsdGeomCamera (lengths in scene units; aperture in tenths of a scene unit as in USD).
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub projection_orthographic: bool,
    pub focal_length: Sampled<f32>,
    /// Apertures and offsets in double precision: USD's fallbacks (20.955, 15.2908)
    /// aren't exact in f32, and renderers derive the field of view from them
    pub horizontal_aperture: f64,
    pub vertical_aperture: f64,
    pub horizontal_aperture_offset: f64,
    pub vertical_aperture_offset: f64,
    pub clipping_range: [f32; 2],
    /// 0 = no depth of field
    pub f_stop: Sampled<f32>,
    pub focus_distance: Sampled<f32>,
    /// `shutter:open` / `shutter:close`, frames relative to the time code
    pub shutter_open: f64,
    pub shutter_close: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            projection_orthographic: false,
            focal_length: Sampled::constant(50.0),
            horizontal_aperture: 20.955,
            vertical_aperture: 15.2908,
            horizontal_aperture_offset: 0.0,
            vertical_aperture_offset: 0.0,
            clipping_range: [1.0, 1_000_000.0],
            f_stop: Sampled::constant(0.0),
            focus_distance: Sampled::constant(0.0),
            shutter_open: 0.0,
            shutter_close: 0.0,
        }
    }
}

/// UsdRenderSettings (the parts a renderer reads), plus other attributes by name.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RenderSettings {
    pub camera: Option<Path>,
    pub resolution: Option<[u32; 2]>,
    pub pixel_aspect_ratio: Option<f32>,
    /// Other settings (`rray:spp`, …) as typed values
    pub attributes: Vec<(String, ShaderValue)>,
}

/// What a prim is.
#[derive(Clone, Debug, PartialEq)]
pub enum PrimKind {
    /// Grouping only (`Scope`, `Xform`, or an untyped prim)
    Group,
    Mesh(Box<Mesh>),
    Curves(Box<Curves>),
    Points(Box<Points>),
    Gprim(Box<Gprim>),
    Instancer(Box<Instancer>),
    /// A native instance of the subtree at `prototype`
    Instance { prototype: Path },
    /// A material; its shader nodes live inside its network, not as prims of their own
    Material(Box<Material>),
    Light(Box<Light>),
    Camera(Box<Camera>),
    RenderSettings(Box<RenderSettings>),
}

/// UsdGeomImageable `purpose`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Purpose {
    #[default]
    Default,
    Render,
    Proxy,
    Guide,
}

/// A prim: its kind, transform, and the inherited-style attributes renderers use.
#[derive(Clone, Debug, PartialEq)]
pub struct Prim {
    pub path: Path,
    /// USD type name (`Xform`, `Scope`, `Mesh`, …), for display
    pub type_name: String,
    pub kind: PrimKind,
    /// Local transform (resolved xformOps), time-sampled
    pub local_xform: Sampled<Mat4d>,
    /// `!resetXformStack!`: ignore the parent transforms
    pub reset_xform_stack: bool,
    pub visible: Sampled<bool>,
    pub purpose: Purpose,
    /// `material:binding` target (resolved by inheritance at sync)
    pub material_binding: Option<Path>,
    /// Shadow opacity override (`primvars:rray:opaque`); `None` = inherited / setting
    pub opaque: Option<bool>,
    /// Primvars of prims whose kind has no list of its own (gprims: `displayColor`, …)
    pub primvars: Vec<Primvar>,
}

impl Prim {
    pub fn new(path: Path, kind: PrimKind) -> Prim {
        Prim {
            path,
            type_name: String::new(),
            kind,
            local_xform: Sampled::default(),
            reset_xform_stack: false,
            visible: Sampled::default(),
            purpose: Purpose::Default,
            material_binding: None,
            opaque: None,
            primvars: Vec::new(),
        }
    }

    /// Local transform at time `t` (identity if unauthored).
    pub fn local_xform_at(&self, t: f64) -> Mat4d {
        self.local_xform.at(t).unwrap_or(Mat4d::IDENTITY)
    }

    /// Visibility at time `t` (visible if unauthored).
    pub fn visible_at(&self, t: f64) -> bool {
        self.visible.at(t).unwrap_or(true)
    }
}
