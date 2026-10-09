//! Export edits as a USD override layer (`.usda`): a layer that sublayers the original
//! stage and holds only `over` opinions (plus new `def` prims for rray's camera and
//! render settings). Opening it gives the full scene with the edits on top; the
//! original files are never modified.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use rray_render::shading::Material;

use crate::MaterialSource;

/// A value written on an attribute.
#[derive(Clone, Debug, PartialEq)]
pub enum ExportValue {
    Float(f32),
    Color([f32; 3]),
    Bool(bool),
}

impl ExportValue {
    fn usda_type(&self) -> &'static str {
        match self {
            ExportValue::Float(_) => "float",
            ExportValue::Color(_) => "color3f",
            ExportValue::Bool(_) => "bool",
        }
    }

    fn usda(&self) -> String {
        match self {
            ExportValue::Float(f) => fmt_f(*f as f64),
            ExportValue::Color(c) => format!("({}, {}, {})", fmt_f(c[0] as f64), fmt_f(c[1] as f64), fmt_f(c[2] as f64)),
            ExportValue::Bool(b) => (if *b { "1" } else { "0" }).to_string(),
        }
    }
}

/// Shortest exact decimal for a value (no exponent, always a decimal point-free
/// form USD reads, e.g. `0.8`, `1`, `0.0000001`).
fn fmt_f(v: f64) -> String {
    if v.is_finite() { format!("{v}") } else { "0".to_string() }
}

#[derive(Default)]
struct Spec {
    /// `Some(type)` = defined here (`def Type "name"`); `None` = an `over`
    def_type: Option<String>,
    lines: Vec<String>,
}

/// An override layer being assembled.
pub struct OverrideLayer {
    source: PathBuf,
    up_axis: String,
    meters_per_unit: Option<f64>,
    time_range: Option<(f64, f64)>,
    render_settings: Option<String>,
    prims: BTreeMap<String, Spec>,
    /// Edits that couldn't be written (kept in the file as comments)
    notes: Vec<String>,
}

impl OverrideLayer {
    /// A layer over `source`, carrying the stage metadata a root layer must hold
    /// (stage-level metadata is only read from the root layer, so without it a Z-up
    /// stage would open Y-up).
    pub fn new(source: &Path, up_axis: &str, meters_per_unit: Option<f64>, time_range: Option<(f64, f64)>) -> Self {
        OverrideLayer {
            source: source.to_path_buf(),
            up_axis: up_axis.to_string(),
            meters_per_unit,
            time_range,
            render_settings: None,
            prims: BTreeMap::new(),
            notes: Vec::new(),
        }
    }

    /// Author an attribute line (`decl` like `token visibility`) on a prim.
    pub fn set(&mut self, prim: &str, decl: &str, value: &str) {
        self.prims.entry(prim.to_string()).or_default().lines.push(format!("{decl} = {value}"));
    }

    /// Define a new prim of `type_name` (its ancestors become `over`s unless defined).
    pub fn define(&mut self, prim: &str, type_name: &str) {
        self.prims.entry(prim.to_string()).or_default().def_type = Some(type_name.to_string());
    }

    /// A note written into the layer as a comment (edits that couldn't be written).
    pub fn note(&mut self, note: &str) {
        self.notes.push(note.to_string());
    }

    pub fn is_empty(&self) -> bool {
        self.prims.is_empty()
    }

    /// Hide a prim (`visibility = "invisible"`).
    pub fn hide(&mut self, prim: &str) {
        self.set(prim, "token visibility", "\"invisible\"");
    }

    /// A shader input value.
    pub fn shader_input(&mut self, shader: &str, input: &str, value: &ExportValue) {
        self.set(shader, &format!("{} inputs:{input}", value.usda_type()), &value.usda());
    }

    /// A camera at `path` looking along `forward` (stage coordinates), with a
    /// vertical field of view and an image aspect (width / height).
    pub fn camera(&mut self, path: &str, eye: [f64; 3], forward: [f64; 3], up: [f64; 3], vfov_deg: f64, aspect: f64) {
        let norm = |v: [f64; 3]| {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
            [v[0] / l, v[1] / l, v[2] / l]
        };
        let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let f = norm(forward);
        let r = norm(cross(f, up));
        let u = cross(r, f);
        let back = [-f[0], -f[1], -f[2]];
        // USD cameras look down -Z with +Y up; rows (row-vector convention): X, Y, Z, origin
        let row = |v: [f64; 3], w: f64| format!("({}, {}, {}, {})", fmt_f(v[0]), fmt_f(v[1]), fmt_f(v[2]), fmt_f(w));
        let m = format!("( {}, {}, {}, {} )", row(r, 0.0), row(u, 0.0), row(back, 0.0), row(eye, 1.0));
        // 36 mm wide film back; the vertical aperture follows the image aspect
        let h_ap = 36.0;
        let v_ap = h_ap / aspect.max(1e-6);
        let focal = (v_ap * 0.5) / (vfov_deg.to_radians() * 0.5).tan().max(1e-6);
        self.define(path, "Camera");
        self.set(path, "float focalLength", &fmt_f(focal));
        self.set(path, "float horizontalAperture", &fmt_f(h_ap));
        self.set(path, "float verticalAperture", &fmt_f(v_ap));
        self.set(path, "float2 clippingRange", "(0.1, 10000000)");
        self.set(path, "matrix4d xformOp:transform", &m);
        self.set(path, "uniform token[] xformOpOrder", "[\"xformOp:transform\"]");
    }

    /// A RenderSettings prim (made the stage's `renderSettingsPrimPath`) with the
    /// resolution, camera and rray's settings (`crate::RRAY_SETTINGS`).
    pub fn render_settings(&mut self, path: &str, camera: &str, resolution: [u32; 2], s: &rray_render::render::RenderSettings) {
        self.define(path, "RenderSettings");
        self.set(path, "rel camera", &format!("<{camera}>"));
        self.set(path, "uniform int2 resolution", &format!("({}, {})", resolution[0], resolution[1]));
        self.set(path, "uniform float pixelAspectRatio", "1");
        let b = |v: bool| if v { "1" } else { "0" };
        self.set(path, "custom uniform int rray:samplesPerPixel", &s.spp.to_string());
        self.set(path, "custom uniform int rray:maxBounces", &s.max_bounces.to_string());
        self.set(path, "custom uniform bool rray:pathRegularization", b(s.regularize));
        self.set(path, "custom uniform bool rray:transmissiveShadows", b(s.transmissive_shadows));
        self.set(path, "custom uniform float rray:indirectClamp", &fmt_f(s.clamp as f64));
        self.set(path, "custom uniform token rray:sampler", &format!("\"{}\"", crate::sampler_token(s.sampler)));
        self.set(path, "custom uniform token rray:pixelFilter", &format!("\"{}\"", crate::filter_token(s.filter)));
        self.set(path, "custom uniform float rray:filterWidth", &fmt_f(s.filter_width as f64));
        self.set(path, "custom uniform int rray:lightSamples", &s.light_samples.to_string());
        self.set(path, "custom uniform int rray:envLightSamples", &s.ibl_samples.to_string());
        self.render_settings = Some(path.to_string());
    }

    /// The layer's text, for writing at `out` (the sublayer path is made relative to it).
    pub fn to_usda(&self, out: &Path) -> String {
        let mut s = String::from("#usda 1.0\n(\n");
        let name = self.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let _ = writeln!(s, "    doc = \"rray overrides of {name}\"");
        let _ = writeln!(s, "    subLayers = [\n        @{}@\n    ]", sublayer_path(&self.source, out));
        let _ = writeln!(s, "    upAxis = \"{}\"", if self.up_axis.eq_ignore_ascii_case("Z") { "Z" } else { "Y" });
        if let Some(m) = self.meters_per_unit {
            let _ = writeln!(s, "    metersPerUnit = {}", fmt_f(m));
        }
        if let Some((a, b)) = self.time_range {
            let _ = writeln!(s, "    startTimeCode = {}\n    endTimeCode = {}", fmt_f(a), fmt_f(b));
        }
        if let Some(rs) = &self.render_settings {
            let _ = writeln!(s, "    renderSettingsPrimPath = \"{rs}\"");
        }
        s.push_str(")\n");
        if !self.notes.is_empty() {
            s.push_str("\n# rray: edits that couldn't be written to this layer:\n");
            for n in &self.notes {
                let _ = writeln!(s, "#   - {}", n.replace('\n', " "));
            }
        }

        // Every edited prim and its ancestors, as a tree
        let mut all: BTreeMap<String, ()> = BTreeMap::new();
        for p in self.prims.keys() {
            let mut cur = p.as_str();
            while !cur.is_empty() && cur != "/" {
                all.insert(cur.to_string(), ());
                cur = match cur.rfind('/') {
                    Some(0) | None => "",
                    Some(i) => &cur[..i],
                };
            }
        }
        let children = |parent: &str| -> Vec<String> {
            all.keys()
                .filter(|k| {
                    let par = match k.rfind('/') {
                        Some(0) => "/",
                        Some(i) => &k[..i],
                        None => "",
                    };
                    par == parent
                })
                .cloned()
                .collect()
        };
        fn emit(s: &mut String, layer: &OverrideLayer, path: &str, depth: usize, children: &dyn Fn(&str) -> Vec<String>) {
            let ind = "    ".repeat(depth);
            let name = path.rsplit('/').next().unwrap_or(path);
            let spec = layer.prims.get(path);
            match spec.and_then(|sp| sp.def_type.as_deref()) {
                Some(t) => {
                    let _ = writeln!(s, "\n{ind}def {t} \"{name}\"");
                }
                None => {
                    let _ = writeln!(s, "\n{ind}over \"{name}\"");
                }
            }
            let _ = writeln!(s, "{ind}{{");
            if let Some(sp) = spec {
                for l in &sp.lines {
                    let _ = writeln!(s, "{ind}    {l}");
                }
            }
            for c in children(path) {
                emit(s, layer, &c, depth + 1, children);
            }
            let _ = writeln!(s, "{ind}}}");
        }
        for root in children("/") {
            emit(&mut s, self, &root, 0, &children);
        }
        s
    }

    pub fn write(&self, out: &Path) -> Result<(), String> {
        std::fs::write(out, self.to_usda(out)).map_err(|e| format!("{}: {e}", out.display()))
    }
}

/// The source stage as seen from the output's folder: relative when both share a
/// root, absolute otherwise; forward slashes.
fn sublayer_path(source: &Path, out: &Path) -> String {
    let abs = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let (src, base) = (abs(source), abs(out.parent().unwrap_or(Path::new("."))));
    let sc: Vec<Component> = src.components().collect();
    let bc: Vec<Component> = base.components().collect();
    let common = sc.iter().zip(&bc).take_while(|(a, b)| a == b).count();
    let s = if common == 0 {
        src.to_string_lossy().into_owned()
    } else {
        let mut rel = PathBuf::new();
        for _ in common..bc.len() {
            rel.push("..");
        }
        for c in &sc[common..] {
            rel.push(c.as_os_str());
        }
        let r = rel.to_string_lossy().into_owned();
        if r.starts_with("..") { r } else { format!("./{r}") }
    };
    s.replace('\\', "/")
}

/// The shader inputs to write for a material edit (`before` = as imported, `after` =
/// as edited), and notes for edits that can't be written.
pub fn material_overrides(src: &MaterialSource, before: &Material, after: &Material) -> (Vec<(String, ExportValue)>, Vec<String>) {
    let mut out: Vec<(String, ExportValue)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let near = |a: f32, b: f32| (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0);
    let changed_f = |a: f32, b: f32| !near(a, b);
    let changed_c = |a: [f32; 3], b: [f32; 3]| (0..3).any(|i| !near(a[i], b[i]));
    let id = src.shader_id.as_str();
    let kind = if id.contains("open_pbr_surface") {
        0
    } else if id.contains("standard_surface") {
        1
    } else if id == "UsdPreviewSurface" {
        2
    } else {
        3
    };
    // Every field the editor can change, by name, for the "not expressible" notes
    macro_rules! fields {
        ($m:expr, $f:ident) => {
            $f!($m, base_weight, F); $f!($m, base_color, C); $f!($m, base_metalness, F); $f!($m, base_diffuse_roughness, F);
            $f!($m, specular_weight, F); $f!($m, specular_color, C); $f!($m, specular_roughness, F);
            $f!($m, specular_roughness_anisotropy, F); $f!($m, specular_ior, F);
            $f!($m, transmission_weight, F); $f!($m, transmission_color, C); $f!($m, transmission_depth, F);
            $f!($m, transmission_scatter, C); $f!($m, transmission_scatter_anisotropy, F);
            $f!($m, transmission_dispersion_scale, F); $f!($m, transmission_dispersion_abbe_number, F);
            $f!($m, subsurface_weight, F); $f!($m, subsurface_color, C); $f!($m, subsurface_radius, F);
            $f!($m, subsurface_radius_scale, C); $f!($m, subsurface_scatter_anisotropy, F);
            $f!($m, coat_weight, F); $f!($m, coat_color, C); $f!($m, coat_roughness, F); $f!($m, coat_roughness_anisotropy, F);
            $f!($m, coat_ior, F); $f!($m, coat_darkening, F);
            $f!($m, fuzz_weight, F); $f!($m, fuzz_color, C); $f!($m, fuzz_roughness, F);
            $f!($m, thin_film_weight, F); $f!($m, thin_film_thickness, F); $f!($m, thin_film_ior, F);
            $f!($m, emission_luminance, F); $f!($m, emission_color, C); $f!($m, geometry_opacity, F);
        };
    }
    // Which OpenPBR fields changed
    let mut changed: Vec<&'static str> = Vec::new();
    macro_rules! diff {
        ($m:expr, $field:ident, F) => {
            if changed_f(before.$field, after.$field) {
                changed.push(stringify!($field));
            }
        };
        ($m:expr, $field:ident, C) => {
            if changed_c(before.$field, after.$field) {
                changed.push(stringify!($field));
            }
        };
    }
    fields!((), diff);
    if before.geometry_thin_walled != after.geometry_thin_walled {
        changed.push("geometry_thin_walled");
    }
    if changed.is_empty() {
        return (out, notes);
    }
    // OpenPBR field → (USD input, value) for this shader type; None = not expressible
    let value_of = |field: &str| -> Option<(String, ExportValue)> {
        let m = after;
        let f = |v: f32| ExportValue::Float(v);
        let c = |v: [f32; 3]| ExportValue::Color(v);
        match kind {
            0 => Some((
                field.to_string(),
                match field {
                    "base_color" => c(m.base_color),
                    "specular_color" => c(m.specular_color),
                    "transmission_color" => c(m.transmission_color),
                    "transmission_scatter" => c(m.transmission_scatter),
                    "subsurface_color" => c(m.subsurface_color),
                    "subsurface_radius_scale" => c(m.subsurface_radius_scale),
                    "coat_color" => c(m.coat_color),
                    "fuzz_color" => c(m.fuzz_color),
                    "emission_color" => c(m.emission_color),
                    "geometry_thin_walled" => ExportValue::Bool(m.geometry_thin_walled),
                    "base_weight" => f(m.base_weight),
                    "base_metalness" => f(m.base_metalness),
                    "base_diffuse_roughness" => f(m.base_diffuse_roughness),
                    "specular_weight" => f(m.specular_weight),
                    "specular_roughness" => f(m.specular_roughness),
                    "specular_roughness_anisotropy" => f(m.specular_roughness_anisotropy),
                    "specular_ior" => f(m.specular_ior),
                    "transmission_weight" => f(m.transmission_weight),
                    "transmission_depth" => f(m.transmission_depth),
                    "transmission_scatter_anisotropy" => f(m.transmission_scatter_anisotropy),
                    "transmission_dispersion_scale" => f(m.transmission_dispersion_scale),
                    "transmission_dispersion_abbe_number" => f(m.transmission_dispersion_abbe_number),
                    "subsurface_weight" => f(m.subsurface_weight),
                    "subsurface_radius" => f(m.subsurface_radius),
                    "subsurface_scatter_anisotropy" => f(m.subsurface_scatter_anisotropy),
                    "coat_weight" => f(m.coat_weight),
                    "coat_roughness" => f(m.coat_roughness),
                    "coat_roughness_anisotropy" => f(m.coat_roughness_anisotropy),
                    "coat_ior" => f(m.coat_ior),
                    "coat_darkening" => f(m.coat_darkening),
                    "fuzz_weight" => f(m.fuzz_weight),
                    "fuzz_roughness" => f(m.fuzz_roughness),
                    "thin_film_weight" => f(m.thin_film_weight),
                    "thin_film_thickness" => f(m.thin_film_thickness),
                    "thin_film_ior" => f(m.thin_film_ior),
                    "emission_luminance" => f(m.emission_luminance),
                    "geometry_opacity" => f(m.geometry_opacity),
                    _ => return None,
                },
            )),
            1 => Some(match field {
                // Inverse of the import's standard_surface → OpenPBR mapping
                "base_weight" => ("base".into(), f(m.base_weight)),
                "base_color" => ("base_color".into(), c(m.base_color)),
                "base_metalness" => ("metalness".into(), f(m.base_metalness)),
                "base_diffuse_roughness" => ("diffuse_roughness".into(), f(m.base_diffuse_roughness)),
                "specular_weight" => ("specular".into(), f(m.specular_weight)),
                "specular_color" => ("specular_color".into(), c(m.specular_color)),
                "specular_roughness" => ("specular_roughness".into(), f(m.specular_roughness)),
                "specular_roughness_anisotropy" => ("specular_anisotropy".into(), f(m.specular_roughness_anisotropy)),
                "specular_ior" => ("specular_IOR".into(), f(m.specular_ior)),
                "transmission_weight" => ("transmission".into(), f(m.transmission_weight)),
                "transmission_color" => ("transmission_color".into(), c(m.transmission_color)),
                "transmission_depth" => ("transmission_depth".into(), f(m.transmission_depth)),
                "transmission_dispersion_abbe_number" => ("transmission_dispersion".into(), f(m.transmission_dispersion_abbe_number)),
                "subsurface_weight" => ("subsurface".into(), f(m.subsurface_weight)),
                "subsurface_color" => ("subsurface_color".into(), c(m.subsurface_color)),
                "coat_weight" => ("coat".into(), f(m.coat_weight)),
                "coat_color" => ("coat_color".into(), c(m.coat_color)),
                "coat_roughness" => ("coat_roughness".into(), f(m.coat_roughness)),
                "coat_ior" => ("coat_IOR".into(), f(m.coat_ior)),
                "fuzz_weight" => ("sheen".into(), f(m.fuzz_weight)),
                "fuzz_color" => ("sheen_color".into(), c(m.fuzz_color)),
                "fuzz_roughness" => ("sheen_roughness".into(), f(m.fuzz_roughness)),
                "thin_film_thickness" | "thin_film_weight" => {
                    let nm = if m.thin_film_weight > 0.0 { m.thin_film_thickness * 1000.0 } else { 0.0 };
                    ("thin_film_thickness".into(), f(nm))
                }
                "thin_film_ior" => ("thin_film_IOR".into(), f(m.thin_film_ior)),
                "emission_luminance" => ("emission".into(), f(m.emission_luminance)),
                "emission_color" => ("emission_color".into(), c(m.emission_color)),
                "geometry_opacity" => ("opacity".into(), c([m.geometry_opacity; 3])),
                "geometry_thin_walled" => ("thin_walled".into(), ExportValue::Bool(m.geometry_thin_walled)),
                _ => return None,
            }),
            2 => Some(match field {
                // Inverse of the import's UsdPreviewSurface → OpenPBR mapping
                "base_color" => ("diffuseColor".into(), c(m.base_color)),
                "base_metalness" => ("metallic".into(), f(m.base_metalness)),
                "specular_roughness" => ("roughness".into(), f(m.specular_roughness)),
                "specular_ior" => ("ior".into(), f(m.specular_ior)),
                "coat_weight" => ("clearcoat".into(), f(m.coat_weight)),
                "coat_roughness" => ("clearcoatRoughness".into(), f(m.coat_roughness)),
                "transmission_weight" => ("opacity".into(), f(1.0 - m.transmission_weight)),
                "emission_color" | "emission_luminance" => {
                    ("emissiveColor".into(), c(m.emission_color.map(|x| x * m.emission_luminance)))
                }
                _ => return None,
            }),
            _ => None,
        }
    };
    if kind == 3 {
        notes.push(format!(
            "{}: edits to `{}` materials can't be written back yet ({} parameter(s) changed)",
            src.material,
            if id.is_empty() { "(no info:id)" } else { id },
            changed.len()
        ));
        return (out, notes);
    }
    for field in changed {
        match value_of(field) {
            Some((input, value)) => {
                if src.connected.iter().any(|c| *c == input) {
                    notes.push(format!("{}: `inputs:{input}` is driven by a texture / node connection; edit not written", src.material));
                } else if !out.iter().any(|(i, _)| *i == input) {
                    out.push((input, value));
                }
            }
            None => notes.push(format!("{}: `{field}` has no `{id}` input; edit not written", src.material)),
        }
    }
    (out, notes)
}
