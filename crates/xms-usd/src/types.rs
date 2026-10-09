//! Public result types of a USD import.

use crate::*;

pub fn is_usd_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("usd" | "usda" | "usdc" | "usdz")
    )
}

// ---------------------------------------------------------------------------
// Public result types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct UsdCamera {
    pub name: String,
    /// The same camera at shutter close, when it moves during the shutter (motion
    /// blur); this one is then at shutter open
    pub at_close: Option<Box<UsdCamera>>,
    pub eye: P3,
    /// Camera basis in world space (−Z view direction, +Y up, +X right; roll preserved)
    pub forward: V3,
    pub right: V3,
    pub up: V3,
    pub vfov_deg: f32,
    pub focus_distance: Option<f32>,
    /// Lens / film back (same units; only ratios matter for a pinhole)
    pub focal_length: f32,
    pub h_aperture: f32,
    pub v_aperture: f32,
    pub h_offset: f32,
    pub v_offset: f32,
    /// Clipping range (near, far)
    pub clipping: [f32; 2],
}

/// The stage's render settings prim (UsdRender), if any.
#[derive(Clone, Debug, Default)]
pub struct UsdRenderSettings {
    pub path: String,
    pub resolution: Option<[u32; 2]>,
    /// Camera prim path from the `camera` relationship
    pub camera: Option<String>,
    /// rray's own settings authored on the prim (`rray:samplesPerPixel`, …): see
    /// [`crate::apply_render_settings`]
    pub rray: Vec<(String, xms_scene::ShaderValue)>,
}

#[derive(Clone, Debug)]
pub struct DomeInfo {
    /// Resolved texture path on disk, if any
    pub texture: Option<PathBuf>,
    /// Authored asset path (for diagnostics)
    pub texture_authored: Option<String>,
    pub color: [f32; 3],
    /// intensity * 2^exposure
    pub intensity: f32,
    /// Environment → world rotation, row-major 3×3 (dome transform, poleAxis applied)
    pub orientation: [f32; 9],
}

/// One prim in the composed stage hierarchy (arena node; parents always precede children).
#[derive(Clone, Debug)]
pub struct UsdNode {
    pub name: String,
    pub path: String,
    pub type_name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub flags: NodeFlags,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NodeFlags {
    /// Instanceable prim whose contents come from a shared prototype
    pub instance: bool,
    /// Lives under an instance (read from the prototype)
    pub instance_proxy: bool,
    /// Prototype of a PointInstancer (not drawn directly)
    pub prototype: bool,
    pub inactive: bool,
    pub invisible: bool,
    /// guide / proxy purpose (skipped by the renderer)
    pub non_render_purpose: bool,
    /// Shading network / other non-renderable prim shown for completeness
    pub display_only: bool,
}

/// Material and hierarchy node of each part of a prototype, as used by a set of
/// instances (instances with the same inherited binding share one of these).
#[derive(Clone, Debug)]
pub struct UsdBinding {
    /// Part index → material index (into `UsdImport::materials`)
    pub materials: Vec<u32>,
    /// Part index → hierarchy node (for visibility and per-prim stats)
    pub nodes: Vec<usize>,
}

/// Geometry of one (non-instanced) prim with one material, built as its own BLAS
/// in the prim's local space and placed by `xform`.
pub struct UsdChunk {
    pub node: usize,
    pub material: usize,
    pub blas: Arc<Blas>,
    /// Object→world (row-major 3×4); None = identity
    pub xform: Option<[f32; 12]>,
    /// World transforms across the shutter when it moves (motion blur)
    pub motion: Option<Arc<rray_render::geom::MotionKeys>>,
}

#[derive(Default, Clone, Debug)]
pub struct UsdStats {
    /// Curve prims imported and their stored segments (each prototype counted once)
    pub curve_prims: usize,
    pub curve_segments: usize,
    /// Meshes refined as subdivision surfaces, how many at each level, and faces produced
    pub subdivided: usize,
    pub subdiv_levels: [usize; 7],
    pub subdiv_faces: usize,
    pub prims: usize,
    pub meshes: usize,
    pub gprims: usize,
    pub instances: usize,
    pub point_instancers: usize,
    /// Render geometry produced (triangles + analytic spheres), after instancing expansion.
    /// Not to be confused with USD *prims* (scene-graph nodes), counted in `prims`.
    pub triangles: usize,
    pub materials: usize,
    pub textures: usize,
    pub texture_bytes: usize,
    /// Triangles actually stored (flat + each prototype once)
    pub unique_triangles: usize,
    /// Renderer instances (native instances + points + nested, flattened)
    pub render_instances: usize,
    pub load_ms: f64,
}

/// Where a material came from in USD: what an override layer edits.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialSource {
    /// The USD `Material` prim
    pub material: String,
    /// Its surface shader prim
    pub shader: String,
    /// The shader's `info:id` (`ND_open_pbr_surface_surfaceshader`, `UsdPreviewSurface`, …)
    pub shader_id: String,
    /// Inputs driven by a connection (texture or other node), by name without `inputs:`
    pub connected: Vec<String>,
}

pub struct UsdImport {
    pub name: String,
    pub path: PathBuf,
    /// Stage hierarchy; index 0 is the pseudo-root "/"
    pub nodes: Vec<UsdNode>,
    pub chunks: Vec<UsdChunk>,
    /// (display name, material); indexed by `UsdChunk::material`
    pub materials: Vec<(String, Material)>,
    /// Per material (aligned with `materials`): its USD source, when it has one
    pub material_sources: Vec<Option<MaterialSource>>,
    /// The shutter the scene was translated with (offsets from `time`; equal = an instant)
    pub shutter: (f64, f64),
    /// Moving instances: (index in `instances`, world transforms across the shutter)
    pub instance_motion: Vec<(u32, Arc<rray_render::geom::MotionKeys>)>,
    /// Texture/node network per material (same indexing as `materials`)
    pub networks: Vec<Option<Arc<ShadingNetwork>>>,
    /// Unique prototypes, each built once as a BLAS in its local space
    pub prototypes: Vec<Arc<Blas>>,
    pub prototype_names: Vec<String>,
    /// Placed prototypes (native instances, PointInstancer points, nested instances flattened)
    pub instances: Vec<InstanceDesc>,
    /// `InstanceDesc::map` indexes this
    pub bindings: Vec<UsdBinding>,
    pub lights: Vec<LightDesc>,
    pub cameras: Vec<UsdCamera>,
    pub dome: Option<DomeInfo>,
    pub render_settings: Option<UsdRenderSettings>,
    pub stats: UsdStats,
    pub warnings: Vec<String>,
    pub up_axis: String,
    pub meters_per_unit: Option<f64>,
    pub time_range: Option<(f64, f64)>,
    pub time: Option<f64>,
}

/// Which dome-space axis the latlong map's pole is mapped to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DomePole {
    /// USD spec: +Y for DomeLight; DomeLight_1 uses poleAxis ("scene" = stage up axis)
    #[default]
    Auto,
    /// Always the dome's +Y (no up-axis correction; what a pipeline that ignores
    /// poleAxis produces on a Z-up stage)
    Y,
    /// Always the dome's +Z
    Z,
}

/// What happens to the composed USD stage (and other import data) once the
/// import is done. Freeing the Moana island's stage took 50 s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StageRelease {
    /// Free it after the result is handed over: the stage is parked on the loading
    /// thread and freed when that thread exits (the GUI loads on a short-lived
    /// thread). Import leftovers are freed on a background thread.
    #[default]
    Background,
    /// Never free it: the process exits soon after (headless `rray`), and the
    /// OS reclaims the memory at once
    Keep,
}

/// (Not `Copy`: it holds the camera path.)
#[derive(Clone, Debug)]
pub struct UsdOptions {
    /// Time code to evaluate at. `None` = stage start time (or default values).
    pub time: Option<f64>,
    /// Load texture pixels during import. When false (the fast default), image
    /// nodes only record which file they need; the app loads them on demand.
    pub load_textures: bool,
    /// Stop importing geometry after this many triangles (memory guard). 0 = unlimited.
    pub max_triangles: usize,
    /// Cap on the per-mesh Catmull-Clark level (0 = never subdivide). Each tagged mesh
    /// gets its own level (authored, else adaptive); this only limits it.
    pub subdiv_max_level: u32,
    /// DomeLight pole handling (default: follow the USD spec)
    pub dome_pole: DomePole,
    /// Import BasisCurves (off = skip them, e.g. to compare load / render cost)
    pub import_curves: bool,
    /// Releasing the stage after import
    pub release: StageRelease,
    /// Motion-blur shutter as offsets from the frame (USD `shutter:open` / `close`).
    /// `None`: read from the render camera (`camera`, else the RenderSettings camera)
    pub shutter: Option<(f64, f64)>,
    /// The camera that will render (a prim path), for reading its shutter
    pub camera: Option<String>,
    /// The renderer's working colour space: colour inputs (materials, textures,
    /// primvars, lights) are converted into it at import
    pub working_space: rray_color::ColorSpace,
    /// Curve / points primvars to capture: `None` = all, `Some(prefix)` = the standard
    /// ones plus those named `prefix…` (default `user:`)
    pub curve_primvars: Option<String>,
}

impl Default for UsdOptions {
    fn default() -> Self {
        Self {
            time: None,
            load_textures: false,
            max_triangles: 0,
            subdiv_max_level: 3,
            dome_pole: DomePole::Auto,
            import_curves: true,
            release: StageRelease::Background,
            shutter: None,
            camera: None,
            working_space: rray_color::ColorSpace::AcesCg,
            curve_primvars: Some(crate::to_scene::CURVE_PRIMVAR_PREFIX.to_string()),
        }
    }
}
