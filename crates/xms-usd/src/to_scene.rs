//! USD stage → [`xms_scene::Scene`]: captures raw, USD-shaped data (no
//! triangulation, subdivision or baking; that's the renderer's sync), with the
//! time samples a shutter window needs. The counterpart of Hydra's UsdImaging.

use std::collections::HashSet;
use std::path::Path as FsPath;
use std::time::Instant;

use openusd::sdf::{Value, ValueKind};
use openusd::usd::{Attribute, Prim, Stage, TimeCode};
use xms_scene as rs;
use xms_scene::{Mat4d, Path, PrimKind, Sampled};

use crate::values::{binding, int_array, numeric_values, scalar, tok, type_of, vec3_array};
use crate::xform::{op_matrix, quat_components};
use crate::M4;

/// What to translate.
#[derive(Clone, Debug)]
pub struct SceneOptions {
    /// Time code (default: the stage's start time, else the default value)
    pub time: Option<f64>,
    /// Shutter interval as offsets from `time` (frames). (0, 0) = one instant
    pub shutter: (f64, f64),
    /// Import BasisCurves
    pub import_curves: bool,
    /// Which primvars curves and points capture: `None` = all of them; `Some(prefix)`
    /// = the standard ones (widths, normals, texture coordinates, displayColor /
    /// displayOpacity) plus those named `prefix…` (e.g. `primvars:user:mask` with
    /// `user:`, kept as `user:mask`). Grooms often carry many per-point attributes
    /// nothing renders; a prefix keeps them from being decoded
    pub curve_primvars: Option<String>,
}

/// The default curve-primvar prefix.
pub const CURVE_PRIMVAR_PREFIX: &str = "user:";

/// Where scene-layer translation spent its time (reported as an import note).
#[derive(Default)]
struct Profile {
    curve_points: std::time::Duration,
    curve_widths_normals: std::time::Duration,
    curve_primvars: std::time::Duration,
    curve_primvars_read: usize,
    meshes: std::time::Duration,
    /// Sampled attributes: openusd decoding vs. rray's conversion
    decode: std::time::Duration,
    convert: std::time::Duration,
    /// Curve / points primvars not captured: name → prims
    skipped: std::collections::BTreeMap<String, usize>,
}

impl Default for SceneOptions {
    fn default() -> Self {
        SceneOptions { time: None, shutter: (0.0, 0.0), import_curves: true, curve_primvars: Some(CURVE_PRIMVAR_PREFIX.to_string()) }
    }
}

/// A translated stage.
pub struct Translated {
    pub scene: rs::Scene,
    pub warnings: Vec<String>,
    pub seconds: f64,
}

/// Open `path` and translate the whole stage.
pub fn translate(path: &FsPath, opts: SceneOptions) -> Result<Translated, String> {
    let path_str = path.to_str().ok_or("Non-UTF-8 path")?;
    let stage = Stage::open(path_str).map_err(|e| format!("Failed to open stage: {e}"))?;
    translate_stage(&stage, path, opts)
}

/// Translate an already-open stage (`path` is its root layer, for `.mtlx` references).
pub fn translate_stage(stage: &Stage, path: &FsPath, opts: SceneOptions) -> Result<Translated, String> {
    let t0 = Instant::now();
    let path_str = path.to_str().ok_or("Non-UTF-8 path")?;
    let z_up = stage.stage_metadata("upAxis").ok().flatten().and_then(|v| tok(&v)).is_some_and(|a| a.eq_ignore_ascii_case("Z"));
    let meters_per_unit = stage.stage_metadata("metersPerUnit").ok().flatten().and_then(scalar);
    let time_range = stage.has_authored_time_code_range().then(|| (stage.start_time_code(), stage.end_time_code()));
    let time = opts.time.or(time_range.map(|r| r.0));
    let t = time.unwrap_or(0.0);
    let mut tr = Translator {
        stage,
        time,
        window: (t + opts.shutter.0, t + opts.shutter.1),
        import_curves: opts.import_curves,
        scene: rs::Scene::new(),
        warnings: Vec::new(),
        prototypes: HashSet::new(),
        mtlx_refs: crate::materialx::scan_mtlx_references(path),
        unresolved_materials: 0,
        proto_problems: (0, 0, 0),
        rebase: None,
        rebased_protos: 0,
        in_proto: 0,
        proto_drops: (0, 0, 0),
        proto_stack: Vec::new(),
        proxy_active: 0,
        curve_primvars: opts.curve_primvars.clone(),
        profile: std::cell::RefCell::new(Profile::default()),
    };
    tr.scene.info = rs::StageInfo { z_up, meters_per_unit, time_range, source: Some(path_str.to_string()) };
    tr.scene.set_time(rs::SceneTime { time: t, shutter_open: opts.shutter.0, shutter_close: opts.shutter.1 });
    let roots = stage.root_prims().map_err(|e| format!("Can't list root prims: {e}"))?;
    // Root prims come back as names
    for name in roots {
        if let Ok(prim) = stage.prim(format!("/{}", name.as_str()).as_str()) {
            tr.walk(&prim);
        }
    }
    {
        // How many prototypes actually hold children in the scene layer
        let root = rs::Path::root();
        let protos: Vec<rs::Path> = tr.scene.children(&root).iter().filter(|c| c.name().starts_with("__Prototype")).cloned().collect();
        let filled = protos.iter().filter(|p| !tr.scene.children(p).is_empty()).count();
        let (inactive, typed, path) = tr.proto_drops;
        tr.warnings.push(format!(
            "Prototypes in the scene layer: {} ({filled} with children) · prims dropped inside prototypes: {inactive} inactive/abstract, {typed} shader/subset type, {path} unparsable path · {} kept because their instance proxy is active",
            protos.len(),
            tr.proxy_active
        ));
        let (ok, empty, failed) = tr.proto_problems;
        if ok + empty + failed > 0 {
            tr.warnings.push(format!(
                "Native-instance prototypes: {ok} translated with children, {empty} opened but empty, {failed} failed to open; \
                 {} reported their prims under an instance path (filed under the prototype path)",
                tr.rebased_protos
            ));
        }
    }
    if tr.unresolved_materials > 0 {
        tr.warnings.push(format!("{} material(s) without a resolvable surface shader", tr.unresolved_materials));
    }
    let Translator { mut scene, mut warnings, profile, .. } = tr;
    // Where the time went
    {
        let p = profile.into_inner();
        let total = t0.elapsed();
        let known = p.curve_points + p.curve_widths_normals + p.curve_primvars + p.meshes;
        let mut note = format!(
            "Scene-layer translation: curve points {:.2} s · curve widths / normals {:.2} s · other curve primvars {:.2} s ({} read) · meshes {:.2} s · everything else {:.2} s · (all sampled attributes: decoding {:.2} s, converting {:.2} s)",
            p.curve_points.as_secs_f64(),
            p.curve_widths_normals.as_secs_f64(),
            p.curve_primvars.as_secs_f64(),
            p.curve_primvars_read,
            p.meshes.as_secs_f64(),
            total.saturating_sub(known).as_secs_f64(),
            p.decode.as_secs_f64(),
            p.convert.as_secs_f64()
        );
        if !p.skipped.is_empty() {
            let names: Vec<&str> = p.skipped.keys().map(String::as_str).take(12).collect();
            let more = p.skipped.len().saturating_sub(names.len());
            note.push_str(&format!(
                " · curve primvars not captured (not standard, not prefixed): {}{}",
                names.join(", "),
                if more > 0 { format!(" (+{more} more)") } else { String::new() }
            ));
        }
        warnings.push(note);
    }
    // The translation itself isn't a change to sync incrementally
    let _ = scene.take_changes();
    Ok(Translated { scene, warnings, seconds: t0.elapsed().as_secs_f64() })
}

struct Translator<'s> {
    stage: &'s Stage,
    time: Option<f64>,
    window: (f64, f64),
    import_curves: bool,
    scene: rs::Scene,
    warnings: Vec<String>,
    /// Native-instance prototypes already translated
    prototypes: HashSet<String>,
    /// Materials defined by referenced .mtlx documents: path → (file, target)
    mtlx_refs: crate::HashMap<String, (String, String)>,
    /// Materials whose surface couldn't be found (diagnostics for the first few)
    unresolved_materials: usize,
    /// Native prototypes: (translated with children, opened but empty, failed to open)
    proto_problems: (usize, usize, usize),
    /// While translating a prototype whose prims report paths elsewhere (openusd
    /// composes prototypes in place, under a canonical instance): rewrite paths
    /// starting with `.0` to start with `.1` (the `/__Prototype_N` path)
    rebase: Option<(String, String)>,
    /// Prototypes whose prims reported paths outside the prototype
    rebased_protos: usize,
    /// Depth inside prototype translation, and prims dropped there by reason:
    /// (inactive or abstract, shader / node graph / subset type, unparsable path)
    in_proto: usize,
    proto_drops: (usize, usize, usize),
    /// Prototypes being translated: (prototype root path, path of the instance that
    /// led to it), innermost last. Used to see a prim as its instance proxy
    proto_stack: Vec<(String, String)>,
    /// Prototype prims kept because their instance proxy is active
    proxy_active: usize,
    curve_primvars: Option<String>,
    profile: std::cell::RefCell<Profile>,
}

fn not_block(v: Option<Value>) -> Option<Value> {
    match v {
        Some(Value::ValueBlock) | Some(Value::None) => None,
        other => other,
    }
}

fn path_of(prim: &Prim) -> Option<Path> {
    Path::new(&prim.path().to_string()).ok()
}

/// nalgebra column-vector matrix → USD row-vector `Mat4d`.
fn to_mat4d(m: &M4) -> Mat4d {
    Mat4d(std::array::from_fn(|i| std::array::from_fn(|j| m[(j, i)])))
}

fn to_u32s(v: Vec<i64>) -> Vec<u32> {
    v.into_iter().map(|x| x.max(0) as u32).collect()
}

/// Authored interpolation token; `None` when unauthored (or unknown).
fn interpolation(t: Option<String>) -> Option<rs::Interpolation> {
    match t.as_deref() {
        Some("constant") => Some(rs::Interpolation::Constant),
        Some("uniform") => Some(rs::Interpolation::Uniform),
        Some("varying") => Some(rs::Interpolation::Varying),
        Some("vertex") => Some(rs::Interpolation::Vertex),
        Some("faceVarying") => Some(rs::Interpolation::FaceVarying),
        _ => None,
    }
}

fn primvar_values(v: Value) -> Option<rs::PrimvarValues> {
    // The common array types straight to their final form (no padded intermediate;
    // a float array is moved as is), large ones in parallel
    use rayon::prelude::*;
    let big = |n: usize| n >= crate::values::PAR_CONVERT;
    match v {
        Value::FloatVec(a) => return Some(rs::PrimvarValues::Float(a)),
        Value::DoubleVec(a) if big(a.len()) => return Some(rs::PrimvarValues::Float(a.par_iter().map(|&x| x as f32).collect())),
        Value::DoubleVec(a) => return Some(rs::PrimvarValues::Float(a.iter().map(|&x| x as f32).collect())),
        Value::Vec2fVec(a) if big(a.len()) => return Some(rs::PrimvarValues::Float2(a.par_iter().map(|p| [p.x, p.y]).collect())),
        Value::Vec2fVec(a) => return Some(rs::PrimvarValues::Float2(a.iter().map(|p| [p.x, p.y]).collect())),
        Value::Vec3fVec(a) if big(a.len()) => return Some(rs::PrimvarValues::Float3(a.par_iter().map(|p| [p.x, p.y, p.z]).collect())),
        Value::Vec3fVec(a) => return Some(rs::PrimvarValues::Float3(a.iter().map(|p| [p.x, p.y, p.z]).collect())),
        Value::Vec3dVec(a) if big(a.len()) => {
            return Some(rs::PrimvarValues::Float3(a.par_iter().map(|p| [p.x as f32, p.y as f32, p.z as f32]).collect()))
        }
        Value::Vec3dVec(a) => return Some(rs::PrimvarValues::Float3(a.iter().map(|p| [p.x as f32, p.y as f32, p.z as f32]).collect())),
        other => primvar_values_generic(other),
    }
}

/// Other kinds (scalars, ints, halves, doubles of other widths, …) through the
/// padded numeric reader.
fn primvar_values_generic(v: Value) -> Option<rs::PrimvarValues> {
    let (dims, vals) = numeric_values(v)?;
    Some(match dims {
        1 => rs::PrimvarValues::Float(vals.iter().map(|x| x[0]).collect()),
        2 => rs::PrimvarValues::Float2(vals.iter().map(|x| [x[0], x[1]]).collect()),
        3 => rs::PrimvarValues::Float3(vals.iter().map(|x| [x[0], x[1], x[2]]).collect()),
        _ => rs::PrimvarValues::Float4(vals),
    })
}

/// A shader input value as a scene value (tokens, strings and assets become text).
fn shader_value(v: Value) -> Option<rs::ShaderValue> {
    if let Some(s) = tok(&v) {
        return Some(rs::ShaderValue::String(s));
    }
    Some(match v {
        Value::AssetPath(ap) => {
            if ap.is_empty() {
                return None;
            }
            rs::ShaderValue::Asset(ap.resolved_path().map(str::to_string).unwrap_or_else(|| ap.asset_path().to_string()))
        }
        Value::Bool(b) => rs::ShaderValue::Bool(b),
        Value::Int(i) => rs::ShaderValue::Int(i),
        Value::Int64(i) => rs::ShaderValue::Int(i as i32),
        Value::Float(f) => rs::ShaderValue::Float(f),
        Value::Double(d) => rs::ShaderValue::Float(d as f32),
        other => {
            let (dims, vals) = numeric_values(other)?;
            let x = *vals.first()?;
            match dims {
                1 => rs::ShaderValue::Float(x[0]),
                2 => rs::ShaderValue::Vec2([x[0], x[1]]),
                3 => rs::ShaderValue::Vec3([x[0], x[1], x[2]]),
                _ => rs::ShaderValue::Vec4(x),
            }
        }
    })
}

/// Where a connected attribute's value comes from.
enum Source {
    /// Output `output` of the shader at `node`
    Node(String, String),
    /// A value (e.g. a material interface input)
    Value(Value),
    None,
}

impl<'s> Translator<'s> {
    // ----- attribute access ------------------------------------------------

    /// Value at the translation time (default value, then earliest sample, when no time).
    fn value(&self, prim: &Prim, name: &str) -> Option<Value> {
        let a = prim.attribute(name);
        not_block(match self.time {
            Some(t) => a.get_at::<Value>(TimeCode::new(t)).ok().flatten().or_else(|| a.get::<Value>().ok().flatten()),
            None => a.get::<Value>().ok().flatten().or_else(|| a.get_at::<Value>(TimeCode::EARLIEST).ok().flatten()),
        })
    }

    fn value_at(&self, prim: &Prim, name: &str, t: f64) -> Option<Value> {
        let a = prim.attribute(name);
        not_block(a.get_at::<Value>(TimeCode::new(t)).ok().flatten().or_else(|| a.get::<Value>().ok().flatten()))
    }

    /// Sample times of an animated attribute covering the shutter window
    /// (samples inside it plus the bracketing ones); empty when it's static.
    fn sample_times(&self, attr: &Attribute) -> Vec<f64> {
        if !attr.value_might_be_time_varying().unwrap_or(false) {
            return Vec::new();
        }
        let (open, close) = self.window;
        let mut times = attr.time_samples_in_interval(open..=close).unwrap_or_default();
        for t in [open, close] {
            if let Ok(Some((lo, hi))) = attr.bracketing_time_samples(TimeCode::new(t)) {
                times.push(lo);
                times.push(hi);
            }
        }
        times.sort_by(f64::total_cmp);
        times.dedup();
        times
    }

    /// An attribute as time samples over the shutter window, converted by `f`
    /// (one sample when it's static).
    fn sampled<T: Clone>(&self, prim: &Prim, name: &str, f: impl Fn(Value) -> Option<T>) -> Option<Sampled<T>> {
        let attr = prim.attribute(name);
        let times = self.sample_times(&attr);
        let (mut dec, mut conv) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        let mut timed = |get: &dyn Fn() -> Option<Value>| -> Option<T> {
            let t0 = std::time::Instant::now();
            let v = get();
            let t1 = std::time::Instant::now();
            let out = v.and_then(&f);
            dec += t1 - t0;
            conv += t1.elapsed();
            out
        };
        let samples: Vec<(f64, T)> = if times.len() > 1 {
            times.into_iter().filter_map(|t| timed(&|| not_block(attr.get_at::<Value>(TimeCode::new(t)).ok().flatten())).map(|x| (t, x))).collect()
        } else {
            timed(&|| self.value(prim, name)).map(|x| vec![(self.time.unwrap_or(0.0), x)]).unwrap_or_default()
        };
        {
            let mut p = self.profile.borrow_mut();
            p.decode += dec;
            p.convert += conv;
        }
        (!samples.is_empty()).then(|| Sampled::from_samples(samples))
    }

    fn scalar_of(&self, prim: &Prim, names: &[&str], default: f64) -> f64 {
        names.iter().find_map(|n| self.value(prim, n).and_then(scalar)).unwrap_or(default)
    }

    fn token_of(&self, prim: &Prim, name: &str) -> Option<String> {
        self.value(prim, name).and_then(|v| tok(&v))
    }

    fn sampled_f32(&self, prim: &Prim, names: &[&str], default: f32) -> Sampled<f32> {
        names.iter().find_map(|n| self.sampled(prim, n, |v| scalar(v).map(|x| x as f32))).unwrap_or_else(|| Sampled::constant(default))
    }

    // ----- transforms --------------------------------------------------------

    /// Local transform from xformOps, sampled at the union of the ops' sample
    /// times in the window, and the `resetXformStack` flag.
    fn local_xform(&self, prim: &Prim) -> (Sampled<Mat4d>, bool) {
        let ops: Vec<String> = match self.value(prim, "xformOpOrder") {
            Some(Value::TokenVec(v)) => v.iter().map(|t| t.as_str().to_string()).collect(),
            Some(Value::StringVec(v)) => v,
            _ => return (Sampled::default(), false),
        };
        let reset = ops.iter().any(|o| o == "!resetXformStack!");
        let mut times: Vec<f64> = Vec::new();
        for op in &ops {
            let name = op.strip_prefix("!invert!").unwrap_or(op);
            if name != "!resetXformStack!" {
                times.extend(self.sample_times(&prim.attribute(name)));
            }
        }
        times.sort_by(f64::total_cmp);
        times.dedup();
        let eval = |t: Option<f64>| -> M4 {
            let mut m = M4::identity();
            for op in &ops {
                if op == "!resetXformStack!" {
                    m = M4::identity();
                    continue;
                }
                let (inverse, name) = match op.strip_prefix("!invert!") {
                    Some(n) => (true, n),
                    None => (false, op.as_str()),
                };
                let val = match t {
                    Some(t) => self.value_at(prim, name, t),
                    None => self.value(prim, name),
                };
                let Some(val) = val else { continue };
                let kind = name.split(':').nth(1).unwrap_or("");
                let Some(mut om) = op_matrix(kind, val) else { continue };
                if inverse {
                    om = om.try_inverse().unwrap_or(om);
                }
                m *= om;
            }
            m
        };
        let samples = if times.len() > 1 {
            times.into_iter().map(|t| (t, to_mat4d(&eval(Some(t))))).collect()
        } else {
            vec![(self.time.unwrap_or(0.0), to_mat4d(&eval(None)))]
        };
        (Sampled::from_samples(samples), reset)
    }

    // ----- walk --------------------------------------------------------------

    /// Whether a prim inside a prototype is active as seen through its instance:
    /// its path is mapped back through the enclosing instances, innermost first.
    fn proxy_is_active(&self, prim: &Prim) -> bool {
        let mut s = prim.path().to_string();
        for (root, instance) in self.proto_stack.iter().rev() {
            if let Some(rest) = s.strip_prefix(root.as_str()).filter(|r| r.is_empty() || r.starts_with('/')) {
                s = format!("{instance}{rest}");
            }
        }
        if s == prim.path().to_string() {
            return false;
        }
        self.stage.prim(s.as_str()).ok().and_then(|p| p.is_active().ok()).unwrap_or(false)
    }

    /// Note a prim dropped while translating a prototype (diagnostics).
    fn proto_drop(&mut self, prim: &Prim, reason: usize, why: String) {
        if self.in_proto == 0 {
            return;
        }
        let n = match reason {
            0 => &mut self.proto_drops.0,
            1 => &mut self.proto_drops.1,
            _ => &mut self.proto_drops.2,
        };
        *n += 1;
        if *n == 1 {
            self.warnings.push(format!("First prototype prim dropped ({why}): {} type `{}`", prim.path(), type_of(prim)));
        }
    }

    fn walk(&mut self, prim: &Prim) {
        let active = prim.is_active();
        let abstract_ = prim.is_abstract();
        let mut is_active = active.as_ref().map_or(true, |a| *a);
        if !is_active && !self.proto_stack.is_empty() {
            // openusd can report prims in the prototype namespace as inactive where
            // the instance proxy (what the direct import reads) is active: ask the proxy
            if self.proxy_is_active(prim) {
                is_active = true;
                self.proxy_active += 1;
            }
        }
        if !is_active || abstract_.as_ref().map_or(false, |a| *a) {
            let why = format!("is_active {:?}, is_abstract {:?}", active.as_ref().ok(), abstract_.as_ref().ok());
            self.proto_drop(prim, 0, why);
            return;
        }
        let Some(path) = self.scene_path(prim) else {
            self.proto_drop(prim, 2, "path not accepted by the scene layer".into());
            return;
        };
        let ty = type_of(prim);
        if matches!(ty.as_str(), "Shader" | "NodeGraph" | "GeomSubset") {
            self.proto_drop(prim, 1, format!("type {ty}"));
            return; // captured by their material / mesh
        }
        let mut descend = true;
        let kind = if prim.is_instance().unwrap_or(false) {
            descend = false; // instance proxies: the prototype is translated once
            // `prototype()` gives the prototype's path (`/__Prototype_N`)
            match prim.prototype() {
                Ok(Some(proto)) => {
                    let proto_path = proto.to_string();
                    if self.prototypes.insert(proto_path.clone()) {
                        match self.stage.prim(proto_path.as_str()) {
                            Ok(proto_prim) => {
                                let n = proto_prim.children().map(|c| c.len());
                                self.walk_prototype(&proto_prim, &prim.path().to_string());
                                if !matches!(n, Ok(k) if k > 0) {
                                    self.proto_problems.1 += 1;
                                    if self.proto_problems.1 <= 3 {
                                        let inst_children = prim.children().map_or_else(|e| format!("error {e}"), |c| c.len().to_string());
                                        let n = n.map_or_else(|e| format!("error {e}"), |k| k.to_string());
                                        self.warnings.push(format!(
                                            "Prototype {proto_path} (of instance {}) opened but lists {n} children; the instance itself lists {inst_children}",
                                            prim.path()
                                        ));
                                    }
                                } else {
                                    self.proto_problems.0 += 1;
                                }
                            }
                            Err(e) => {
                                self.proto_problems.2 += 1;
                                if self.proto_problems.2 <= 3 {
                                    let inst_children = prim.children().map_or_else(|e| format!("error {e}"), |c| c.len().to_string());
                                    self.warnings.push(format!(
                                        "Prototype {proto_path} (of instance {}) can't be opened by path: {e}; the instance itself lists {inst_children} children",
                                        prim.path()
                                    ));
                                }
                            }
                        }
                    }
                    match Path::new(&proto_path) {
                        Ok(p) => PrimKind::Instance { prototype: p },
                        Err(_) => PrimKind::Group,
                    }
                }
                _ => PrimKind::Group,
            }
        } else {
            match ty.as_str() {
                "Mesh" => {
                    let t = std::time::Instant::now();
                    let k = self.mesh(prim).map_or(PrimKind::Group, |m| PrimKind::Mesh(Box::new(m)));
                    self.profile.borrow_mut().meshes += t.elapsed();
                    k
                }
                "BasisCurves" if self.import_curves => self.curves(prim).map_or(PrimKind::Group, |c| PrimKind::Curves(Box::new(c))),
                "Points" => self.points(prim).map_or(PrimKind::Group, |p| PrimKind::Points(Box::new(p))),
                "Sphere" | "Cube" | "Cylinder" | "Cylinder_1" | "Cone" | "Capsule" | "Capsule_1" | "Plane" => {
                    PrimKind::Gprim(Box::new(self.gprim(prim, &ty)))
                }
                "PointInstancer" => PrimKind::Instancer(Box::new(self.instancer(prim))),
                "Material" => {
                    descend = false;
                    let m = self.material(prim);
                    if m.surface.is_none() && m.mtlx_document.is_none() {
                        self.unresolved_materials += 1;
                        if self.unresolved_materials <= 3 {
                            let d = self.material_diagnostic(prim);
                            self.warnings.push(d);
                        }
                    }
                    PrimKind::Material(Box::new(m))
                }
                "DistantLight" | "SphereLight" | "RectLight" | "DiskLight" | "CylinderLight" | "DomeLight" | "DomeLight_1" => {
                    PrimKind::Light(Box::new(self.light(prim, &ty)))
                }
                "Camera" => PrimKind::Camera(Box::new(self.camera(prim))),
                "RenderSettings" => PrimKind::RenderSettings(Box::new(self.render_settings(prim))),
                _ => PrimKind::Group,
            }
        };
        let mut p = rs::Prim::new(path, kind);
        p.type_name = ty.clone();
        if matches!(p.kind, PrimKind::Gprim(_)) {
            p.primvars = self.primvars(prim, &[]);
        }
        let (xf, reset) = self.local_xform(prim);
        p.local_xform = xf;
        p.reset_xform_stack = reset;
        p.visible = self.sampled(prim, "visibility", |v| tok(&v).map(|t| t != "invisible")).unwrap_or_default();
        p.purpose = match self.token_of(prim, "purpose").as_deref() {
            Some("render") => rs::Purpose::Render,
            Some("proxy") => rs::Purpose::Proxy,
            Some("guide") => rs::Purpose::Guide,
            _ => rs::Purpose::Default,
        };
        p.material_binding = binding(prim).and_then(|b| self.target_path(&b));
        p.opaque = ["primvars:rray:opaque", "rray:opaque", "primvars:arnold:opaque"]
            .iter()
            .find_map(|n| self.value(prim, n).and_then(scalar))
            .map(|v| v != 0.0);
        self.scene.insert(p);
        if descend {
            for c in prim.children().unwrap_or_default() {
                self.walk(&c);
            }
        }
    }

    /// A native-instance prototype (`/__Prototype_N`) and its subtree, once.
    fn walk_prototype(&mut self, proto: &Prim, instance: &str) {
        let Some(path) = path_of(proto) else { return };
        self.proto_stack.push((path.as_str().to_string(), instance.to_string()));
        self.scene.insert(rs::Prim::new(path.clone(), PrimKind::Group));
        let children = proto.children().unwrap_or_default();
        // openusd may report a prototype's prims under the canonical instance's path;
        // file them under the prototype path, where instances (and the mapping of
        // instance-proxy paths) look for them
        let reported_parent = children.first().map(|c| c.path().to_string()).and_then(|p| p.rsplit_once('/').map(|(a, _)| a.to_string()));
        let saved = self.rebase.take();
        if let Some(rp) = reported_parent.filter(|rp| !rp.is_empty() && rp != path.as_str()) {
            self.rebased_protos += 1;
            self.rebase = Some((rp, path.as_str().to_string()));
        }
        self.in_proto += 1;
        for c in children {
            self.walk(&c);
        }
        self.in_proto -= 1;
        self.rebase = saved;
        self.proto_stack.pop();
    }

    /// The scene path for a prim: its own path, rebased into the prototype being
    /// translated when its prims report paths elsewhere.
    fn scene_path(&self, prim: &Prim) -> Option<Path> {
        self.target_path(&prim.path().to_string())
    }

    /// A path (prim or relationship target) as the scene layer files it, rebased
    /// into the prototype being translated when needed.
    fn target_path(&self, s: &str) -> Option<Path> {
        if let Some((from, to)) = &self.rebase {
            if s == from {
                return Path::new(to).ok();
            }
            if let Some(rest) = s.strip_prefix(from.as_str()).filter(|r| r.starts_with('/')) {
                return Path::new(&format!("{to}{rest}")).ok();
            }
        }
        Path::new(s).ok()
    }

    // ----- geometry ------------------------------------------------------------

    fn primvar(&self, prim: &Prim, attr: &str, name: &str) -> Option<rs::Primvar> {
        let values = self.sampled(prim, attr, primvar_values)?;
        let interp = prim.attribute(attr).get_metadata::<Value>("interpolation").ok().flatten().and_then(|v| tok(&v));
        Some(rs::Primvar {
            name: name.to_string(),
            interpolation: interpolation(interp),
            values,
            indices: self.value(prim, &format!("{attr}:indices")).and_then(int_array).map(to_u32s),
        })
    }

    /// Every numeric `primvars:<name>` except normals and renderer-namespaced ones.
    fn primvars(&self, prim: &Prim, skip: &[&str]) -> Vec<rs::Primvar> {
        let mut out = Vec::new();
        for prop in prim.property_names().unwrap_or_default() {
            let full = prop.as_str();
            let Some(name) = full.strip_prefix("primvars:") else { continue };
            if name.contains(':') || skip.contains(&name) {
                continue;
            }
            if let Some(pv) = self.primvar(prim, full, name) {
                out.push(pv);
            }
        }
        out
    }

    fn mesh(&self, prim: &Prim) -> Option<rs::Mesh> {
        let points = self.sampled(prim, "points", vec3_array)?;
        let counts = self.value(prim, "faceVertexCounts").and_then(int_array).map(to_u32s).unwrap_or_default();
        let indices = self.value(prim, "faceVertexIndices").and_then(int_array).map(to_u32s).unwrap_or_default();
        let normals = if prim.attribute("primvars:normals").is_defined().unwrap_or(false) {
            self.primvar(prim, "primvars:normals", "normals")
        } else {
            self.primvar(prim, "normals", "normals")
        };
        let scheme = self.token_of(prim, "subdivisionScheme");
        let level = ["primvars:rray:subdivLevel", "rray:subdivLevel", "primvars:arnold:subdiv_iterations"]
            .iter()
            .find_map(|n| self.value(prim, n).and_then(scalar))
            .map(|v| v.max(0.0).round() as u32);
        let ints = |n: &str| self.value(prim, n).and_then(int_array).map(to_u32s).unwrap_or_default();
        let floats = |n: &str| self.value(prim, n).and_then(crate::values::float_array).unwrap_or_default();
        let subdivision = rs::Subdivision {
            scheme,
            level,
            interpolate_boundary: self.token_of(prim, "interpolateBoundary"),
            crease_indices: ints("creaseIndices"),
            crease_lengths: ints("creaseLengths"),
            crease_sharpnesses: floats("creaseSharpnesses"),
            corner_indices: ints("cornerIndices"),
            corner_sharpnesses: floats("cornerSharpnesses"),
        };
        let mut subsets = Vec::new();
        for c in prim.children().unwrap_or_default() {
            if type_of(&c) != "GeomSubset" {
                continue;
            }
            // Face subsets only (as the importer: any family)
            if self.token_of(&c, "elementType").is_some_and(|e| e != "face") {
                continue;
            }
            subsets.push(rs::GeomSubset {
                name: c.path().to_string(),
                faces: self.value(&c, "indices").and_then(int_array).map(to_u32s).unwrap_or_default(),
                material_binding: binding(&c).and_then(|b| self.target_path(&b)),
            });
        }
        Some(rs::Mesh {
            points,
            face_vertex_counts: counts,
            face_vertex_indices: indices,
            hole_indices: ints("holeIndices"),
            left_handed: self.token_of(prim, "orientation").as_deref() == Some("leftHanded"),
            subdivision,
            normals,
            primvars: self.primvars(prim, &["normals"]),
            subsets,
        })
    }

    fn curves(&self, prim: &Prim) -> Option<rs::Curves> {
        let t = std::time::Instant::now();
        let points = self.sampled(prim, "points", vec3_array)?;
        self.profile.borrow_mut().curve_points += t.elapsed();
        let t = std::time::Instant::now();
        let (widths, normals) = (self.primvar(prim, "widths", "widths"), self.primvar(prim, "normals", "normals"));
        self.profile.borrow_mut().curve_widths_normals += t.elapsed();
        let primvars = self.curve_primvars(prim, &["normals", "widths"]);
        Some(rs::Curves {
            curve_type: self.token_of(prim, "type").unwrap_or_else(|| "cubic".into()),
            basis: self.token_of(prim, "basis").unwrap_or_else(|| "bezier".into()),
            wrap: self.token_of(prim, "wrap").unwrap_or_else(|| "nonperiodic".into()),
            curve_vertex_counts: self.value(prim, "curveVertexCounts").and_then(int_array).map(to_u32s).unwrap_or_default(),
            points,
            widths,
            normals,
            primvars,
        })
    }

    fn points(&self, prim: &Prim) -> Option<rs::Points> {
        let points = self.sampled(prim, "points", vec3_array)?;
        Some(rs::Points {
            points,
            widths: self.primvar(prim, "widths", "widths"),
            primvars: self.curve_primvars(prim, &["widths"]),
        })
    }

    /// Primvars of curves / points (see `SceneOptions::curve_primvars`): all of them,
    /// or the standard ones plus those with the prefix; the others are listed in the
    /// profile, not decoded. Namespaced names (`user:mask`) are kept whole; `…:indices`
    /// attributes belong to their primvar.
    fn curve_primvars(&self, prim: &Prim, skip: &[&str]) -> Vec<rs::Primvar> {
        let t = std::time::Instant::now();
        let standard = |n: &str| crate::curves::CURVE_ST_NAMES.contains(&n) || n == "displayColor" || n == "displayOpacity";
        let mut out = Vec::new();
        for prop in prim.property_names().unwrap_or_default() {
            let full = prop.as_str();
            let Some(name) = full.strip_prefix("primvars:") else { continue };
            if name.ends_with(":indices") || skip.contains(&name) {
                continue;
            }
            let wanted = match &self.curve_primvars {
                None => true,
                Some(prefix) => standard(name) || (!prefix.is_empty() && name.starts_with(prefix.as_str())),
            };
            if wanted {
                if let Some(pv) = self.primvar(prim, full, name) {
                    out.push(pv);
                }
            } else {
                *self.profile.borrow_mut().skipped.entry(name.to_string()).or_default() += 1;
            }
        }
        let mut p = self.profile.borrow_mut();
        p.curve_primvars += t.elapsed();
        p.curve_primvars_read += out.len();
        out
    }

    fn gprim(&self, prim: &Prim, ty: &str) -> rs::Gprim {
        let axis = match self.token_of(prim, "axis").as_deref() {
            Some("X") => rs::Axis::X,
            Some("Y") => rs::Axis::Y,
            _ => rs::Axis::Z,
        };
        match ty {
            "Sphere" => rs::Gprim::Sphere { radius: self.scalar_of(prim, &["radius"], 1.0) },
            "Cube" => rs::Gprim::Cube { size: self.scalar_of(prim, &["size"], 2.0) },
            "Cylinder" | "Cylinder_1" => {
                rs::Gprim::Cylinder { radius: self.scalar_of(prim, &["radius"], 1.0), height: self.scalar_of(prim, &["height"], 2.0), axis }
            }
            "Cone" => rs::Gprim::Cone { radius: self.scalar_of(prim, &["radius"], 1.0), height: self.scalar_of(prim, &["height"], 2.0), axis },
            "Capsule" | "Capsule_1" => {
                rs::Gprim::Capsule { radius: self.scalar_of(prim, &["radius"], 0.5), height: self.scalar_of(prim, &["height"], 1.0), axis }
            }
            _ => rs::Gprim::Plane { width: self.scalar_of(prim, &["width"], 2.0), length: self.scalar_of(prim, &["length"], 2.0), axis },
        }
    }

    fn instancer(&self, prim: &Prim) -> rs::Instancer {
        let prototypes = prim
            .relationship("prototypes")
            .targets()
            .unwrap_or_default()
            .iter()
            .filter_map(|t| self.target_path(&t.to_string()))
            .collect();
        let orientations = ["orientations", "orientationsf"]
            .iter()
            // Authored components, reordered (real, i, j, k) → (i, j, k, real): lossless
            // (narrowing to f32 is exact: the authored values are f32)
            .find_map(|n| {
                self.sampled(prim, n, |v| {
                    quat_components(v).map(|qs| qs.into_iter().map(|q| [q[1] as f32, q[2] as f32, q[3] as f32, q[0] as f32]).collect::<Vec<_>>())
                })
            });
        let ids = |n: &str| self.value(prim, n).and_then(int_array);
        rs::Instancer {
            prototypes,
            proto_indices: self.value(prim, "protoIndices").and_then(int_array).map(to_u32s).unwrap_or_default(),
            positions: self.sampled(prim, "positions", vec3_array).unwrap_or_default(),
            orientations,
            scales: self.sampled(prim, "scales", vec3_array),
            invisible_ids: ids("invisibleIds").unwrap_or_default(),
            ids: ids("ids"),
        }
    }

    // ----- materials -------------------------------------------------------------

    /// Follow a connection to the shader output (or value) it comes from,
    /// through NodeGraph outputs and material / node-graph interface inputs.
    fn resolve(&self, attr: &Attribute, depth: usize) -> Source {
        let Some(c) = attr.connections().unwrap_or_default().first().map(|p| p.to_string()) else { return Source::None };
        let Some((prim_path, prop)) = c.split_once('.') else { return Source::None };
        let Ok(src) = self.stage.prim(prim_path) else { return Source::None };
        // Anything that isn't a NodeGraph or Material is a shader node (as the importer
        // does: some assets type their shaders differently, or not at all)
        let ty = type_of(&src);
        if ty != "NodeGraph" && ty != "Material" && prop.starts_with("outputs:") {
            return Source::Node(prim_path.to_string(), prop.trim_start_matches("outputs:").to_string());
        }
        if depth > 8 {
            return Source::None;
        }
        // NodeGraph output or interface input: follow it, else use its value
        let next = src.attribute(prop);
        match self.resolve(&next, depth + 1) {
            Source::None => self.value(&src, prop).map_or(Source::None, Source::Value),
            found => found,
        }
    }

    fn capture_node(&self, node_path: &str, m: &mut rs::Material, seen: &mut HashSet<String>) {
        if !seen.insert(node_path.to_string()) {
            return;
        }
        let (Ok(node), Ok(path)) = (self.stage.prim(node_path), Path::new(node_path)) else { return };
        let mut inputs = Vec::new();
        let mut upstream = Vec::new();
        for prop in node.property_names().unwrap_or_default() {
            let full = prop.as_str();
            let Some(name) = full.strip_prefix("inputs:") else { continue };
            let attr = node.attribute(full);
            let value = match self.resolve(&attr, 0) {
                Source::Node(src, output) => {
                    upstream.push(src.clone());
                    match Path::new(&src) {
                        Ok(p) => Some(rs::ShaderValue::Connection { node: p, output }),
                        Err(_) => None,
                    }
                }
                Source::Value(v) => shader_value(v),
                Source::None => self.value(&node, full).and_then(shader_value),
            };
            if let Some(value) = value {
                // Color space: only texture `file` inputs carry it (as the importer reads)
                let color_space = (name == "file")
                    .then(|| attr.get_metadata::<Value>("colorSpace").ok().flatten().and_then(|v| tok(&v)))
                    .flatten();
                inputs.push(rs::ShaderInput { name: name.to_string(), value, color_space });
            }
        }
        m.nodes.push(rs::ShaderNode { path, id: self.token_of(&node, "info:id").unwrap_or_default(), type_name: type_of(&node), inputs });
        for u in upstream {
            self.capture_node(&u, m, seen);
        }
    }

    fn material(&self, prim: &Prim) -> rs::Material {
        let mut m = rs::Material::default();
        let mut seen = HashSet::new();
        // Terminals in the importer's order: the universal one, then any render-context
        // `outputs:<context>:<terminal>` (e.g. `outputs:mtlx:surface`, `outputs:ri:surface`)
        // in property order
        let props: Vec<String> = prim.property_names().unwrap_or_default().iter().map(|t| t.as_str().to_string()).collect();
        for (slot, terminal) in ["surface", "displacement", "volume"].into_iter().enumerate() {
            let base = format!("outputs:{terminal}");
            let suffix = format!(":{terminal}");
            let names: Vec<String> = std::iter::once(base.clone())
                .chain(props.iter().filter(|p| p.starts_with("outputs:") && p.ends_with(&suffix) && **p != base).cloned())
                .collect();
            for n in &names {
                if let Source::Node(node, output) = self.resolve(&prim.attribute(n.as_str()), 0) {
                    if let Ok(p) = Path::new(&node) {
                        let t = Some((p, output));
                        match slot {
                            0 => m.surface = t,
                            1 => m.displacement = t,
                            _ => m.volume = t,
                        }
                        self.capture_node(&node, &mut m, &mut seen);
                    }
                    break;
                }
            }
        }
        m.mtlx_document = self.mtlx_refs.get(&prim.path().to_string()).cloned();
        m
    }

    /// Why a material's surface wasn't found: its outputs, their raw connection
    /// targets, and what each target is.
    fn material_diagnostic(&self, prim: &Prim) -> String {
        let mut parts = Vec::new();
        for prop in prim.property_names().unwrap_or_default() {
            let name = prop.as_str();
            if !name.starts_with("outputs:") {
                continue;
            }
            let conns = prim.attribute(name).connections();
            let desc = match conns {
                Err(e) => format!("error {e}"),
                Ok(c) if c.is_empty() => "no connection".to_string(),
                Ok(c) => c
                    .iter()
                    .map(|t| {
                        let s = t.to_string();
                        let target = s.split_once('.').map_or(s.as_str(), |(p, _)| p);
                        let what = match self.stage.prim(target) {
                            Ok(p) => format!("type `{}`, scene path ok: {}", type_of(&p), Path::new(target).is_ok()),
                            Err(e) => format!("not found ({e})"),
                        };
                        format!("`{s}` → {what}")
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            };
            parts.push(format!("{name}: {desc}"));
        }
        if parts.is_empty() {
            parts.push("no outputs".to_string());
        }
        format!("material {} · {}", prim.path(), parts.join(" · "))
    }

    // ----- lights, cameras, settings ----------------------------------------------

    fn light(&self, prim: &Prim, ty: &str) -> rs::Light {
        let p = |n: &str| [format!("inputs:{n}"), n.to_string()];
        let f = |n: &str, d: f64| {
            let [a, b] = p(n);
            self.scalar_of(prim, &[a.as_str(), b.as_str()], d) as f32
        };
        let asset = |n: &str| {
            let [a, b] = p(n);
            [a, b].iter().find_map(|k| self.value(prim, k).and_then(shader_value)).and_then(|v| match v {
                rs::ShaderValue::Asset(s) | rs::ShaderValue::String(s) => Some(s),
                _ => None,
            })
        };
        let kind = match ty {
            "DistantLight" => rs::LightKind::Distant { angle: f("angle", 0.53) },
            "SphereLight" => rs::LightKind::Sphere { radius: f("radius", 0.5), treat_as_point: f("treatAsPoint", 0.0) != 0.0 },
            "RectLight" => rs::LightKind::Rect { width: f("width", 1.0), height: f("height", 1.0), texture: asset("texture:file") },
            "DiskLight" => rs::LightKind::Disk { radius: f("radius", 0.5) },
            "CylinderLight" => {
                rs::LightKind::Cylinder { radius: f("radius", 0.5), length: f("length", 1.0), treat_as_line: f("treatAsLine", 0.0) != 0.0 }
            }
            _ => rs::LightKind::Dome {
                texture: asset("texture:file"),
                format: self.token_of(prim, "inputs:texture:format"),
                pole_axis: self.token_of(prim, "poleAxis"),
            },
        };
        let color = ["inputs:color", "color"]
            .iter()
            .find_map(|n| self.sampled(prim, n, crate::values::vec3))
            .unwrap_or_else(|| Sampled::constant([1.0; 3]));
        let opt = |n: &str| {
            let [a, b] = p(n);
            [a, b].iter().find_map(|k| self.value(prim, k).and_then(scalar)).map(|v| v as f32)
        };
        rs::Light {
            kind,
            intensity: self.sampled_f32(prim, &["inputs:intensity", "intensity"], 1.0),
            exposure: self.sampled_f32(prim, &["inputs:exposure", "exposure"], 0.0),
            color,
            normalize: f("normalize", 0.0) != 0.0,
            enable_color_temperature: f("enableColorTemperature", 0.0) != 0.0,
            color_temperature: f("colorTemperature", 6500.0),
            cone_angle: opt("shaping:cone:angle"),
            cone_softness: opt("shaping:cone:softness"),
            focus: opt("shaping:focus"),
            camera_visible: ["karma:light:renderlightgeo", "inputs:camera:visible", "primvars:visibleInCamera"]
                .iter()
                .find_map(|n| self.value(prim, n).and_then(scalar))
                .is_some_and(|v| v != 0.0),
        }
    }

    fn camera(&self, prim: &Prim) -> rs::Camera {
        // Apertures stay f64: the fallbacks aren't exact in f32 (authored floats widen exactly)
        let f = |n: &str, d: f64| self.scalar_of(prim, &[n], d);
        let clip = self.value(prim, "clippingRange").and_then(numeric_values).and_then(|(_, v)| v.first().map(|x| [x[0], x[1]]));
        let d = rs::Camera::default();
        rs::Camera {
            projection_orthographic: self.token_of(prim, "projection").as_deref() == Some("orthographic"),
            focal_length: self.sampled_f32(prim, &["focalLength"], 50.0),
            horizontal_aperture: f("horizontalAperture", d.horizontal_aperture),
            vertical_aperture: f("verticalAperture", d.vertical_aperture),
            horizontal_aperture_offset: f("horizontalApertureOffset", 0.0),
            vertical_aperture_offset: f("verticalApertureOffset", 0.0),
            clipping_range: clip.unwrap_or(d.clipping_range),
            f_stop: self.sampled_f32(prim, &["fStop"], 0.0),
            focus_distance: self.sampled_f32(prim, &["focusDistance"], 0.0),
            shutter_open: self.scalar_of(prim, &["shutter:open"], 0.0),
            shutter_close: self.scalar_of(prim, &["shutter:close"], 0.0),
        }
    }

    fn render_settings(&self, prim: &Prim) -> rs::RenderSettings {
        let camera = prim.relationship("camera").targets().ok().and_then(|t| t.first().and_then(|p| Path::new(&p.to_string()).ok()));
        let resolution = self
            .value(prim, "resolution")
            .and_then(|v| match v {
                Value::Vec2f(p) => Some([p.x as u32, p.y as u32]),
                other => match other.coerce_to_kind(ValueKind::Vec2f) {
                    Ok(Value::Vec2f(p)) => Some([p.x as u32, p.y as u32]),
                    _ => None,
                },
            })
            .filter(|r| r[0] > 0 && r[1] > 0);
        let mut attributes = Vec::new();
        for prop in prim.property_names().unwrap_or_default() {
            let name = prop.as_str();
            if name.starts_with("rray:") {
                if let Some(v) = self.value(prim, name).and_then(shader_value) {
                    attributes.push((name.to_string(), v));
                }
            }
        }
        rs::RenderSettings {
            camera,
            resolution,
            pixel_aspect_ratio: self.value(prim, "pixelAspectRatio").and_then(scalar).map(|v| v as f32),
            attributes,
        }
    }
}
