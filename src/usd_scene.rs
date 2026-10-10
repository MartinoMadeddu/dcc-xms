//! USD stage reader.
//!
//! Stages are read composed, through `xms-usd` (see `usd_stage`): sublayers,
//! references, payloads, inherits, specializes and variants are resolved, and
//! instancing is kept. The result is a `UsdScene`: meshes, curves and points
//! as geometry in their own space with every primvar, each placed by its
//! world transform (Y-up metres) and with its material binding, plus cameras,
//! materials with their textures, skeletons, lights, time range and the stage
//! hierarchy for the scene explorer.
//!
//! When the composed read fails, or finds no meshes, the root layer alone is
//! read as before (`load_root_layer`): every `xformOpOrder` op, up axis and
//! unit, no composition.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use bevy::math::{Mat4, Quat, Vec3};
use openusd::sdf::{self, AbstractData, Value};

use crate::types::MeshData;

#[derive(Clone, Debug)]
pub struct UsdMesh {
    pub path:     String,
    pub mesh:     Arc<MeshData>,
    /// Path of the bound material, when there is one.
    pub material: Option<String>,
    /// The geometry in its own space, with every primvar: what `mesh` is a
    /// view of. `None` from the root-layer reader.
    pub geo:      Option<Arc<crate::core::geo::Geo>>,
    /// In place, or copies of a mesh shared between instances.
    pub place:    crate::types::Placement,
    /// Its USD purpose (render, proxy, guide, or none).
    pub purpose:  crate::types::Purpose,
}

#[derive(Clone, Debug, Default)]
pub struct UsdCamera {
    pub path:         String,
    pub projection:   String,
    pub focal_length: f32,
    pub aperture:     [f32; 2],
    pub clip:         [f32; 2],
    pub position:     [f32; 3],
}

#[derive(Clone, Debug, Default)]
pub struct UsdMaterial {
    pub path:      String,
    pub diffuse:   Option<[f32; 3]>,
    pub roughness: Option<f32>,
    pub metallic:  Option<f32>,
    pub opacity:   Option<f32>,
    /// Texture files, with the surface input each one feeds.
    pub textures:  Vec<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct UsdSkeleton {
    pub path:   String,
    pub joints: Vec<String>,
}

/// One prim of the composed stage, as the scene explorer lists it.
#[derive(Clone, Debug, Default)]
pub struct StageNode {
    /// Path as the user sees it: under an instance, the instance's own path.
    pub path:      String,
    pub name:      String,
    pub type_name: String,
    /// 1 for a root prim.
    pub depth:     usize,
    /// Invisible: listed, not drawn.
    pub hidden:    bool,
    /// Its own transform, relative to its parent (as authored, f64).
    pub local:     bevy::math::DMat4,
    /// It ignores its parents' transforms (`!resetXformStack!`).
    pub reset_xform: bool,
    /// Inside a native instance: read from the prototype, and read-only in
    /// USD (instance proxies cannot carry opinions).
    pub proxy:     bool,
    /// A point instancer's prototype: drawn through the instancer.
    pub prototype: bool,
}

/// The stage hierarchy, depth first (parents before their children).
/// Shared by every packed primitive that came from the stage.
#[derive(Clone, Default)]
pub struct StageTree {
    pub nodes: Vec<StageNode>,
    /// The stage's up-axis and unit correction, included in every primitive's
    /// placement: what writing takes off again to get back to the file's space.
    pub root:  bevy::math::Mat4,
    /// The file the stage was read from, and its up axis and unit: what an
    /// override layer sublayers and repeats.
    pub source:          std::path::PathBuf,
    pub up_axis:         String,
    pub meters_per_unit: Option<f64>,
}

impl std::fmt::Debug for StageTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StageTree({} prims)", self.nodes.len())
    }
}

#[derive(Clone, Debug, Default)]
pub struct UsdScene {
    /// Folder the layer was read from: texture paths are relative to it.
    pub base_dir:        std::path::PathBuf,
    /// The composed stage's hierarchy. Empty when only the root layer was read.
    pub tree:            Arc<StageTree>,
    pub meshes:          Vec<UsdMesh>,
    pub cameras:         Vec<UsdCamera>,
    pub materials:       Vec<UsdMaterial>,
    pub skeletons:       Vec<UsdSkeleton>,
    /// Path and type of each light.
    pub lights:          Vec<(String, String)>,
    pub up_axis:         String,
    pub meters_per_unit: Option<f64>,
    /// Start, end and time codes per second, when authored.
    pub time:            Option<(f64, f64, f64)>,
    /// How many prims of each type the layer holds.
    pub prim_counts:     BTreeMap<String, usize>,
    /// Attributes that change over time.
    pub animated:        usize,
    /// What the layer holds that was not read in full.
    pub notes:           Vec<String>,
}

impl UsdScene {
    /// Each material as the viewport can show it, by material path.
    pub fn looks(&self) -> std::collections::HashMap<String, Arc<crate::types::Look>> {
        self.materials.iter().map(|m| {
            let file = |role: &str| m.textures.iter().find(|(r, _)| r == role).map(|(_, f)| self.base_dir.join(f));
            let look = crate::types::Look {
                name: m.path.rsplit('/').next().unwrap_or("").to_string(),
                // A textured surface is white under its texture.
                color: m.diffuse.unwrap_or(if file("diffuseColor").is_some() { [1.0; 3] } else { [0.6; 3] }),
                roughness: m.roughness.unwrap_or(0.5),
                metallic: m.metallic.unwrap_or(0.0),
                opacity: m.opacity.unwrap_or(1.0),
                color_map: file("diffuseColor"),
                emissive_map: file("emissiveColor"),
                cutout: m.textures.iter().any(|(r, _)| r == "opacity"),
            };
            (m.path.clone(), Arc::new(look))
        }).collect()
    }

    /// Triangles drawn, every instanced copy counted.
    pub fn triangles(&self) -> usize { self.meshes.iter().map(|m| m.mesh.indices.len() / 3 * m.place.copies()).sum() }
}

// ── Reading values ───────────────────────────────────────────────────────────

fn floats(v: &Value) -> Option<Vec<f64>> {
    Some(match v {
        Value::Float(x) => vec![*x as f64],
        Value::Double(x) => vec![*x],
        Value::Half(x) => vec![x.to_f64()],
        Value::Int(x) => vec![*x as f64],
        Value::FloatVec(a) | Value::Vec2f(a) | Value::Vec3f(a) | Value::Vec4f(a) | Value::Quatf(a) => a.iter().map(|x| *x as f64).collect(),
        Value::DoubleVec(a) | Value::Vec2d(a) | Value::Vec3d(a) | Value::Vec4d(a) | Value::Quatd(a)
            | Value::Matrix2d(a) | Value::Matrix3d(a) | Value::Matrix4d(a) => a.clone(),
        Value::HalfVec(a) | Value::Vec2h(a) | Value::Vec3h(a) | Value::Vec4h(a) | Value::Quath(a) => a.iter().map(|x| x.to_f64()).collect(),
        _ => return None,
    })
}

fn ints(v: &Value) -> Option<Vec<usize>> {
    Some(match v {
        Value::IntVec(a) => a.iter().map(|x| (*x).max(0) as usize).collect(),
        Value::UintVec(a) => a.iter().map(|x| *x as usize).collect(),
        Value::Int64Vec(a) => a.iter().map(|x| (*x).max(0) as usize).collect(),
        Value::Uint64Vec(a) => a.iter().map(|x| *x as usize).collect(),
        _ => return None,
    })
}

fn token(v: &Value) -> Option<String> {
    match v { Value::Token(s) | Value::String(s) | Value::AssetPath(s) => Some(s.clone()), _ => None }
}

fn tokens(v: &Value) -> Option<Vec<String>> {
    match v { Value::TokenVec(a) | Value::StringVec(a) => Some(a.clone()), _ => None }
}

/// A layer of either format.
enum Layer { Binary(Box<dyn AbstractData>), Text(crate::usda_text::TextLayer) }

struct Reader {
    data:     Layer,
    animated: usize,
}

impl Reader {
    fn field(&mut self, path: &str, field: &str) -> Option<Value> {
        match &mut self.data {
            Layer::Text(layer) => layer.get(path, field),
            Layer::Binary(data) => {
                let p = if path == "/" { sdf::Path::abs_root() } else { sdf::path(path).ok()? };
                data.get(&p, field).ok().map(|v| v.into_owned())
            }
        }
    }
    /// Value of an attribute: its default, or its first time sample.
    fn attr(&mut self, prim: &str, name: &str) -> Option<Value> {
        let path = format!("{prim}.{name}");
        if let Some(v) = self.field(&path, "default") {
            if !matches!(v, Value::ValueBlock) { return Some(v); }
        }
        match self.field(&path, "timeSamples") {
            Some(Value::TimeSamples(samples)) => samples.into_iter().next().map(|(_, v)| v),
            _ => None,
        }
    }
    fn is_animated(&mut self, prim: &str, name: &str) -> bool {
        matches!(self.field(&format!("{prim}.{name}"), "timeSamples"), Some(Value::TimeSamples(s)) if s.len() > 1)
    }
    fn children(&mut self, prim: &str) -> Vec<String> {
        self.field(prim, "primChildren").and_then(|v| tokens(&v)).unwrap_or_default()
    }
    fn props(&mut self, prim: &str) -> Vec<String> {
        self.field(prim, "properties").and_then(|v| tokens(&v)).unwrap_or_default()
    }
    /// First target of a relationship or connection.
    fn target(&mut self, prim: &str, name: &str, field: &str) -> Option<String> {
        match self.field(&format!("{prim}.{name}"), field)? {
            Value::PathListOp(op) => op.explicit_items.iter().chain(&op.prepended_items).chain(&op.appended_items).chain(&op.added_items)
                .next().map(|p| p.to_string()),
            _ => None,
        }
    }
}

// ── Transforms ───────────────────────────────────────────────────────────────

/// A flat USD matrix as a bevy matrix. USD writes rows and multiplies row
/// vectors, bevy writes columns and multiplies column vectors: the two
/// differences cancel, so the sixteen numbers are taken as they come.
pub fn mat4_from_usd(v: &[f64]) -> Mat4 {
    if v.len() < 16 { return Mat4::IDENTITY; }
    let mut a = [0.0f32; 16];
    for (o, i) in a.iter_mut().zip(v) { *o = *i as f32; }
    Mat4::from_cols_array(&a)
}

fn rotation(axis: char, degrees: f64) -> Mat4 {
    let r = degrees.to_radians() as f32;
    match axis { 'X' => Mat4::from_rotation_x(r), 'Y' => Mat4::from_rotation_y(r), _ => Mat4::from_rotation_z(r) }
}

/// One transform op as a matrix. `kind` is the part after `xformOp:`.
fn op_matrix(kind: &str, value: &Value) -> Option<Mat4> {
    let v = floats(value)?;
    let v3 = |v: &[f64]| Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32);
    Some(match kind {
        "translate" if v.len() >= 3 => Mat4::from_translation(v3(&v)),
        "scale" if v.len() >= 3 => Mat4::from_scale(v3(&v)),
        "scale" if v.len() == 1 => Mat4::from_scale(Vec3::splat(v[0] as f32)),
        // USD quaternions are written real part first.
        "orient" if v.len() >= 4 => Mat4::from_quat(Quat::from_xyzw(v[1] as f32, v[2] as f32, v[3] as f32, v[0] as f32).normalize()),
        "transform" if v.len() >= 16 => mat4_from_usd(&v),
        "rotateX" | "rotateY" | "rotateZ" if !v.is_empty() => rotation(kind.chars().last()?, v[0]),
        // rotateXYZ turns about X first, then Y, then Z.
        k if k.starts_with("rotate") && k.len() == 9 && v.len() >= 3 => {
            let mut m = Mat4::IDENTITY;
            for (axis, angle) in k[6..].chars().zip(&v) { m = rotation(axis, *angle) * m; }
            m
        }
        _ => return None,
    })
}

/// Local transform of a prim from its `xformOpOrder`, and whether it resets
/// the stack, which cuts the prim loose from its parents.
fn local_xform(r: &mut Reader, prim: &str) -> (Mat4, bool) {
    let Some(order) = r.attr(prim, "xformOpOrder").and_then(|v| tokens(&v)) else { return (Mat4::IDENTITY, false) };
    let (mut local, mut reset) = (Mat4::IDENTITY, false);
    for op in order {
        if op == "!resetXformStack!" { reset = true; local = Mat4::IDENTITY; continue; }
        let (name, invert) = match op.strip_prefix("!invert!") { Some(n) => (n.to_string(), true), None => (op.clone(), false) };
        let kind = name.split(':').nth(1).unwrap_or("").to_string();
        if r.is_animated(prim, &name) { r.animated += 1; }
        let Some(m) = r.attr(prim, &name).and_then(|v| op_matrix(&kind, &v)) else { continue };
        local *= if invert { m.inverse() } else { m };
    }
    (local, reset)
}

// ── Meshes ───────────────────────────────────────────────────────────────────

/// Names tried for the texture coordinates, before any other 2D primvar.
const UV_NAMES: [&str; 5] = ["primvars:st", "primvars:st0", "primvars:UVMap", "primvars:map1", "primvars:uv"];

fn read_mesh(r: &mut Reader, prim: &str, props: &[String], world: &Mat4) -> Option<MeshData> {
    let points = floats(&r.attr(prim, "points")?)?;
    let counts = ints(&r.attr(prim, "faceVertexCounts")?)?;
    let indices = ints(&r.attr(prim, "faceVertexIndices")?)?;
    if points.len() < 9 || indices.is_empty() { return None; }
    if r.is_animated(prim, "points") { r.animated += 1; }
    let vertices: Vec<[f32; 3]> = points.chunks_exact(3).map(|p| {
        world.transform_point3(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)).to_array()
    }).collect();
    // A mirrored transform or a left-handed mesh turns the faces inside out.
    let left_handed = r.attr(prim, "orientation").and_then(|v| token(&v)).as_deref() == Some("leftHanded");
    let flip = left_handed != (world.determinant() < 0.0);

    // Texture coordinates, per vertex or per face corner, indexed or not.
    let uv_name = UV_NAMES.iter().map(|n| n.to_string()).find(|n| props.contains(n)).or_else(|| {
        props.iter().find(|p| p.starts_with("primvars:") && !p.ends_with(":indices")
            && matches!(r.attr(prim, p), Some(Value::Vec2f(_) | Value::Vec2d(_) | Value::Vec2h(_)))).cloned()
    });
    let uv = uv_name.and_then(|name| {
        let values = floats(&r.attr(prim, &name)?)?;
        let index = r.attr(prim, &format!("{name}:indices")).and_then(|v| ints(&v));
        let count = index.as_ref().map(|i| i.len()).unwrap_or(values.len() / 2);
        let stated = r.field(&format!("{prim}.{name}"), "interpolation").and_then(|v| token(&v));
        let per_corner = match stated.as_deref() {
            Some("faceVarying") => true,
            Some("vertex") | Some("varying") => false,
            _ => count == indices.len() && count != vertices.len(),
        };
        Some((values, index, per_corner))
    });
    let uv_at = |corner: usize, vertex: usize| -> [f32; 2] {
        let Some((values, index, per_corner)) = &uv else { return [0.0; 2] };
        let slot = if *per_corner { corner } else { vertex };
        let i = match index { Some(ix) => ix.get(slot).copied().unwrap_or(0), None => slot };
        match values.get(i * 2..i * 2 + 2) { Some(p) => [p[0] as f32, p[1] as f32], None => [0.0; 2] }
    };

    let mut tris: Vec<u32> = Vec::with_capacity(indices.len() * 2);
    let mut uvs: Vec<[f32; 2]> = if uv.is_some() { Vec::with_capacity(indices.len() * 2) } else { vec![] };
    let mut polys: Vec<Vec<u32>> = Vec::with_capacity(counts.len());
    let mut at = 0usize;
    for n in &counts {
        let n = *n;
        if at + n > indices.len() { break; }
        if n >= 3 && indices[at..at + n].iter().all(|i| *i < vertices.len()) {
            // Corners of this face, in the winding the mesh ends up with.
            let corners: Vec<usize> = if flip { (0..n).rev().map(|k| at + k).collect() } else { (0..n).map(|k| at + k).collect() };
            polys.push(corners.iter().map(|c| indices[*c] as u32).collect());
            for k in 1..n - 1 {
                for c in [corners[0], corners[k], corners[k + 1]] {
                    tris.push(indices[c] as u32);
                    if uv.is_some() { uvs.push(uv_at(c, indices[c])); }
                }
            }
        }
        at += n;
    }
    if tris.is_empty() { return None; }
    let mut mesh = MeshData::from_triangles(vertices, tris);
    mesh.face_count = polys.len();
    mesh.polys = polys;
    mesh.uvs = uvs;
    Some(mesh)
}

// ── Materials ────────────────────────────────────────────────────────────────

fn read_material(r: &mut Reader, prim: &str) -> (UsdMaterial, usize) {
    let mut out = UsdMaterial { path: prim.to_string(), ..Default::default() };
    // Shaders anywhere under the material.
    let mut shaders = vec![];
    let mut todo = vec![prim.to_string()];
    while let Some(p) = todo.pop() {
        for c in r.children(&p) {
            let child = format!("{p}/{c}");
            if r.field(&child, "typeName").and_then(|v| token(&v)).as_deref() == Some("Shader") { shaders.push(child.clone()); }
            todo.push(child);
        }
    }
    let file_of = |r: &mut Reader, shader: &str| r.attr(shader, "inputs:file").and_then(|v| token(&v));
    for shader in &shaders {
        if r.attr(shader, "info:id").and_then(|v| token(&v)).as_deref() != Some("UsdPreviewSurface") { continue; }
        let scalar = |r: &mut Reader, name: &str| r.attr(shader, name).and_then(|v| floats(&v)).and_then(|v| v.first().copied()).map(|v| v as f32);
        out.diffuse = r.attr(shader, "inputs:diffuseColor").and_then(|v| floats(&v)).filter(|v| v.len() >= 3).map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]);
        out.roughness = scalar(r, "inputs:roughness");
        out.metallic = scalar(r, "inputs:metallic");
        out.opacity = scalar(r, "inputs:opacity");
        // Inputs wired to a texture: follow the wire to the file.
        for input in r.props(shader) {
            let Some(role) = input.strip_prefix("inputs:") else { continue };
            let Some(source) = r.target(shader, &input, "connectionPaths") else { continue };
            let source_prim = source.split('.').next().unwrap_or("").to_string();
            if let Some(file) = file_of(r, &source_prim) { out.textures.push((role.to_string(), file)); }
        }
    }
    (out, shaders.len())
}

// ── The stage ────────────────────────────────────────────────────────────────

fn read_scene(data: Layer) -> UsdScene {
    let mut r = Reader { data, animated: 0 };
    let mut scene = UsdScene::default();
    scene.up_axis = r.field("/", "upAxis").and_then(|v| token(&v)).unwrap_or_else(|| "Y".into());
    scene.meters_per_unit = r.field("/", "metersPerUnit").and_then(|v| floats(&v)).and_then(|v| v.first().copied());
    let number = |r: &mut Reader, name: &str| r.field("/", name).and_then(|v| floats(&v)).and_then(|v| v.first().copied());
    if let (Some(start), Some(end)) = (number(&mut r, "startTimeCode"), number(&mut r, "endTimeCode")) {
        scene.time = Some((start, end, number(&mut r, "timeCodesPerSecond").unwrap_or(24.0)));
    }
    // To Y-up, and to metres when the layer says what its unit is.
    let mut root = Mat4::IDENTITY;
    if scene.up_axis == "Z" { root = Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2); }
    if let Some(unit) = scene.meters_per_unit { if unit > 0.0 && (unit - 1.0).abs() > 1e-9 { root *= Mat4::from_scale(Vec3::splat(unit as f32)); } }

    let (mut composed, mut instanced, mut subsets) = (0usize, 0usize, 0usize);
    let mut todo: Vec<(String, Mat4)> = vec![("/".to_string(), root)];
    while let Some((parent, parent_world)) = todo.pop() {
        let mut kids = r.children(&parent);
        kids.reverse();   // the stack then pops them in file order
        let mut next = vec![];
        for name in kids {
            let prim = if parent == "/" { format!("/{name}") } else { format!("{parent}/{name}") };
            let kind = r.field(&prim, "typeName").and_then(|v| token(&v)).unwrap_or_default();
            *scene.prim_counts.entry(if kind.is_empty() { "(untyped)".into() } else { kind.clone() }).or_default() += 1;
            for field in ["references", "payload", "inheritPaths", "specializes", "variantSetNames"] {
                if r.field(&prim, field).is_some() { composed += 1; break; }
            }
            if matches!(r.field(&prim, "instanceable"), Some(Value::Bool(true))) { instanced += 1; }
            let (local, reset) = local_xform(&mut r, &prim);
            let world = if reset { root * local } else { parent_world * local };
            let props = r.props(&prim);
            match kind.as_str() {
                "Mesh" => {
                    if let Some(mesh) = read_mesh(&mut r, &prim, &props, &world) {
                        let material = r.target(&prim, "material:binding", "targetPaths");
                        scene.meshes.push(UsdMesh { path: prim.clone(), mesh: Arc::new(mesh), material, geo: None, place: Default::default(), purpose: Default::default() });
                    }
                }
                "Camera" => {
                    let one = |r: &mut Reader, n: &str, d: f32| r.attr(&prim, n).and_then(|v| floats(&v)).and_then(|v| v.first().copied()).map(|v| v as f32).unwrap_or(d);
                    let clip = r.attr(&prim, "clippingRange").and_then(|v| floats(&v)).filter(|v| v.len() >= 2).map(|v| [v[0] as f32, v[1] as f32]).unwrap_or([1.0, 1.0e6]);
                    scene.cameras.push(UsdCamera {
                        path: prim.clone(),
                        projection: r.attr(&prim, "projection").and_then(|v| token(&v)).unwrap_or_else(|| "perspective".into()),
                        focal_length: one(&mut r, "focalLength", 50.0),
                        aperture: [one(&mut r, "horizontalAperture", 20.955), one(&mut r, "verticalAperture", 15.2908)],
                        clip,
                        position: world.transform_point3(Vec3::ZERO).to_array(),
                    });
                }
                "Material" => {
                    let (m, shaders) = read_material(&mut r, &prim);
                    scene.materials.push(m);
                    if shaders > 0 { *scene.prim_counts.entry("Shader".into()).or_default() += shaders; }
                }
                "Skeleton" => {
                    let joints = r.attr(&prim, "joints").and_then(|v| tokens(&v)).unwrap_or_default();
                    scene.skeletons.push(UsdSkeleton { path: prim.clone(), joints });
                }
                "GeomSubset" => subsets += 1,
                "PointInstancer" => instanced += 1,
                k if k.ends_with("Light") => scene.lights.push((prim.clone(), k.to_string())),
                _ => {}
            }
            // Materials are read whole above; nothing under them is geometry.
            if kind != "Material" { next.push((prim, world)); }
        }
        todo.extend(next);
    }
    scene.animated = r.animated;

    let count = |scene: &UsdScene, kind: &str| scene.prim_counts.get(kind).copied().unwrap_or(0);
    if composed > 0 { scene.notes.push(format!("{composed} prims use references, payloads, inherits or variants: other files and variants are not resolved")); }
    if instanced > 0 { scene.notes.push(format!("{instanced} instanced prims or point instancers are not expanded")); }
    if subsets > 0 { scene.notes.push(format!("{subsets} face subsets (per-face materials) are not read")); }
    if scene.animated > 0 { scene.notes.push(format!("{} animated attributes are read at their first sample", scene.animated)); }
    if !scene.skeletons.is_empty() || count(&scene, "SkelAnimation") > 0 { scene.notes.push("Skeletons are listed; skinning and skeletal animation are not applied".into()); }
    for (kind, what) in [("BasisCurves", "curves"), ("Points", "point clouds"), ("NurbsPatch", "NURBS patches"), ("Volume", "volumes")] {
        if count(&scene, kind) > 0 { scene.notes.push(format!("{} {what} are not read", count(&scene, kind))); }
    }
    scene
}

/// Unpack a `.usdz` into a folder of its own in the temp directory and
/// return its first `.usda` or `.usdc`: the root layer. The textures come
/// out beside it. A usdz stores its files uncompressed. Files already
/// unpacked at the right size are left alone.
fn extract_usdz(path: &Path) -> Result<std::path::PathBuf, String> {
    use std::hash::{Hash, Hasher};
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    let dir = std::env::temp_dir().join(format!("xms_usdz_{:016x}", h.finish()));
    let mut layer = None;
    let mut i = 0usize;
    while i + 30 < bytes.len() {
        if &bytes[i..i + 4] != b"PK\x03\x04" { i += 1; continue; }
        let u16_at = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]) as usize;
        let size = u32::from_le_bytes([bytes[i + 18], bytes[i + 19], bytes[i + 20], bytes[i + 21]]) as usize;
        let name_end = i + 30 + u16_at(i + 26);
        let start = name_end + u16_at(i + 28);
        let end = start + size;
        if name_end > bytes.len() || end > bytes.len() { break; }
        let name = String::from_utf8_lossy(&bytes[i + 30..name_end]).to_string();
        i = end.max(i + 1);
        // Nothing that would land outside the folder.
        if name.is_empty() || name.ends_with('/') || name.starts_with('/') || name.split('/').any(|part| part == "..") { continue; }
        let out = dir.join(&name);
        if let Some(parent) = out.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
        let there = std::fs::metadata(&out).map(|m| m.len() as usize == size).unwrap_or(false);
        if !there { std::fs::write(&out, &bytes[start..end]).map_err(|e| e.to_string())?; }
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if layer.is_none() && (ext == "usda" || ext == "usdc" || ext == "usd") { layer = Some(out); }
    }
    layer.ok_or_else(|| "no usda or usdc layer inside the usdz".to_string())
}

fn open(path: &Path) -> Result<Layer, String> {
    // Binary layers start with this tag, whatever the extension says.
    let mut head = [0u8; 8];
    let binary = std::fs::File::open(path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)).is_ok() && &head == b"PXR-USDC";
    if binary {
        openusd::usdc::read_file(path).map(Layer::Binary).map_err(|e| e.to_string())
    } else {
        std::fs::read_to_string(path).map(|text| Layer::Text(crate::usda_text::parse(&text))).map_err(|e| e.to_string())
    }
}

/// Read a USD file. Not cached: see `load_cached`.
pub fn load(path: &Path) -> Result<UsdScene, String> {
    if !path.exists() { return Err(format!("{} not found", path.display())); }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    // A usdz is unpacked first, so its textures are files on disk.
    let layer = if ext == "usdz" { extract_usdz(path)? } else { path.to_path_buf() };
    let base_dir = layer.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let composed = crate::usd_stage::read(&layer);
    let mut scene = match composed {
        Ok(scene) if !scene.meshes.is_empty() => scene,
        other => {
            // Nothing drawn from the composed stage: the root layer on its own
            // may still give meshes (older reader, more lenient).
            let why = match &other { Ok(_) => "no meshes found".to_string(), Err(e) => e.clone() };
            match (load_root_layer(&layer, &ext), other) {
                (Ok(mut root), _) if !root.meshes.is_empty() => {
                    root.notes.insert(0, format!("Read the root layer only, without composition ({why})"));
                    root
                }
                (_, Ok(scene)) => scene,
                (Ok(root), Err(_)) => root,
                (Err(e), Err(_)) => return Err(e),
            }
        }
    };
    scene.base_dir = base_dir;
    // The file as given (a .usdz, not the folder it was unpacked into):
    // what an override layer sublayers.
    std::sync::Arc::make_mut(&mut scene.tree).source = path.to_path_buf();
    Ok(scene)
}

/// The root layer on its own, without composition: the reader used before
/// stages were composed, kept as a fallback.
fn load_root_layer(layer: &Path, ext: &str) -> Result<UsdScene, String> {
    let mut scene = match open(layer) {
        Ok(data) => read_scene(data),
        Err(e) => {
            // The older text reader copes with some files this one refuses.
            let meshes = crate::usd_loader::load_usd_meshes(layer).map_err(|_| e.clone())?;
            let mut scene = UsdScene { up_axis: "Y".into(), ..Default::default() };
            scene.meshes = meshes.into_iter().map(|(path, mesh)| UsdMesh { path, mesh: Arc::new(mesh), material: None, geo: None, place: Default::default(), purpose: Default::default() }).collect();
            scene.notes.push(format!("Read with the fallback text reader, meshes only ({e})"));
            scene
        }
    };
    scene.base_dir = layer.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    if scene.meshes.is_empty() && ext != "usdz" {
        // Nothing found: let the older text reader try.
        if let Ok(meshes) = crate::usd_loader::load_usd_meshes(layer) {
            if !meshes.is_empty() {
                scene.meshes = meshes.into_iter().map(|(path, mesh)| UsdMesh { path, mesh: Arc::new(mesh), material: None, geo: None, place: Default::default(), purpose: Default::default() }).collect();
                scene.notes.push("Read with the fallback text reader, meshes only".into());
            }
        }
    }
    Ok(scene)
}

type Cached = (std::path::PathBuf, Option<std::time::SystemTime>, Result<Arc<UsdScene>, String>);
static CACHE: Mutex<Vec<Cached>> = Mutex::new(Vec::new());

/// Read a USD file once, and again only when the file changes on disk.
pub fn load_cached(path: &str) -> Result<Arc<UsdScene>, String> {
    let path = std::path::PathBuf::from(path);
    let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    if let Ok(cache) = CACHE.lock() {
        if let Some((_, _, scene)) = cache.iter().find(|(p, s, _)| *p == path && *s == stamp) { return scene.clone(); }
    }
    let scene = load(&path).map(Arc::new);
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|(p, _, _)| *p != path);
        if cache.len() >= 6 { cache.remove(0); }
        cache.push((path, stamp, scene.clone()));
    }
    scene
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("xms_usd_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
    /// A primitive as it is drawn: its geometry where its first copy is placed.
    fn world(m: &UsdMesh) -> MeshData {
        match m.place.matrices().first() {
            Some(at) => crate::node_graph::nodes::place_mesh(&m.mesh, at),
            None => (*m.mesh).clone(),
        }
    }
    fn bounds(m: &MeshData) -> (Vec3, Vec3) {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for v in &m.vertices { lo = lo.min(Vec3::from_array(*v)); hi = hi.max(Vec3::from_array(*v)); }
        (lo, hi)
    }
    const QUAD: &str = r#"
            int[] faceVertexCounts = [4]
            int[] faceVertexIndices = [0, 1, 2, 3]
            point3f[] points = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]
"#;

    #[test]
    fn a_matrix_keeps_its_translation() {
        // The translation of a USD matrix is its last row.
        let m = mat4_from_usd(&[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 6.0, 7.0, 1.0]);
        assert_eq!(m.transform_point3(Vec3::ZERO), Vec3::new(5.0, 6.0, 7.0));
        // A quarter turn about Z, written the USD way, takes X to Y.
        let r = mat4_from_usd(&[0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        assert!((r.transform_point3(Vec3::X) - Vec3::Y).length() < 1e-6);
    }

    #[test]
    fn transform_ops_apply_in_order_down_the_hierarchy() {
        let path = write("ops.usda", &format!(r#"#usda 1.0
def Xform "car" {{
    double3 xformOp:translate = (10, 0, 0)
    uniform token[] xformOpOrder = ["xformOp:translate"]
    def Xform "wheel" {{
        matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 2, 0, 1) )
        uniform token[] xformOpOrder = ["xformOp:transform"]
        def Mesh "tyre" {{
            float3 xformOp:rotateXYZ = (0, 0, 90)
            float3 xformOp:scale = (2, 2, 2)
            uniform token[] xformOpOrder = ["xformOp:rotateXYZ", "xformOp:scale"]
{QUAD}        }}
    }}
    def Mesh "loose" {{
        double3 xformOp:translate = (0, 0, 3)
        uniform token[] xformOpOrder = ["!resetXformStack!", "xformOp:translate"]
{QUAD}    }}
}}
"#));
        let scene = load(&path).unwrap();
        assert_eq!(scene.meshes.len(), 2, "{:?}", scene.notes);
        let tyre = scene.meshes.iter().find(|m| m.path.ends_with("tyre")).unwrap();
        // Scaled by two, turned a quarter about Z, then moved by (10, 2, 0).
        let (lo, hi) = bounds(&world(tyre));
        assert!((lo - Vec3::new(8.0, 2.0, 0.0)).length() < 1e-4 && (hi - Vec3::new(10.0, 4.0, 0.0)).length() < 1e-4, "{lo} {hi}");
        // The reset stack ignores the parents.
        let loose = scene.meshes.iter().find(|m| m.path.ends_with("loose")).unwrap();
        assert!((bounds(&world(loose)).0 - Vec3::new(0.0, 0.0, 3.0)).length() < 1e-4);
    }

    #[test]
    fn up_axis_and_unit_become_y_up_metres() {
        let path = write("units.usda", &format!(r#"#usda 1.0
(
    metersPerUnit = 0.01
    upAxis = "Z"
)
def Mesh "m" {{
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0, 1, 2]
    point3f[] points = [(0, 0, 0), (100, 0, 0), (0, 0, 200)]
}}
"#));
        let scene = load(&path).unwrap();
        assert_eq!((scene.up_axis.as_str(), scene.meters_per_unit), ("Z", Some(0.01)));
        let (_, hi) = bounds(&world(&scene.meshes[0]));
        // 200 units up Z is two metres up Y.
        assert!((hi - Vec3::new(1.0, 2.0, 0.0)).length() < 1e-4, "{hi}");
    }

    #[test]
    fn uvs_cameras_and_materials_are_read() {
        let path = write("look.usda", r#"#usda 1.0
def Xform "world" {
    def Mesh "card" {
        int[] faceVertexCounts = [4]
        int[] faceVertexIndices = [0, 1, 2, 3]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]
        texCoord2f[] primvars:st = [(0, 0), (1, 0), (1, 1), (0, 1)] (
            interpolation = "vertex"
        )
        rel material:binding = </world/looks/red>
    }
    def Camera "cam" {
        float focalLength = 35
        float2 clippingRange = (0.1, 500)
        double3 xformOp:translate = (0, 1, 8)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
    def Scope "looks" {
        def Material "red" {
            token outputs:surface.connect = </world/looks/red/surface.outputs:surface>
            def Shader "surface" {
                uniform token info:id = "UsdPreviewSurface"
                color3f inputs:diffuseColor.connect = </world/looks/red/tex.outputs:rgb>
                float inputs:roughness = 0.4
                token outputs:surface
            }
            def Shader "tex" {
                uniform token info:id = "UsdUVTexture"
                asset inputs:file = @textures/red.png@
                float3 outputs:rgb
            }
        }
    }
}
"#);
        let scene = load(&path).unwrap();
        let card = &scene.meshes[0];
        assert_eq!(card.material.as_deref(), Some("/world/looks/red"));
        // Two triangles, a UV per corner, matching the corner positions.
        assert_eq!(card.mesh.uvs.len(), 6);
        for (corner, uv) in card.mesh.uvs.iter().enumerate() {
            let p = card.mesh.vertices[card.mesh.indices[corner] as usize];
            assert_eq!([p[0], p[1]], *uv);
        }
        assert_eq!(scene.cameras.len(), 1);
        assert_eq!((scene.cameras[0].focal_length, scene.cameras[0].position), (35.0, [0.0, 1.0, 8.0]));
        assert_eq!(scene.materials.len(), 1);
        assert_eq!(scene.materials[0].roughness, Some(0.4));
        assert_eq!(scene.materials[0].textures, vec![("diffuseColor".to_string(), "textures/red.png".to_string())]);
        assert_eq!(scene.prim_counts.get("Shader"), Some(&2));
    }

    #[test]
    fn the_example_files_load() {
        for (file, meshes) in [(crate::examples::SHAPES_USD, 3), (crate::examples::TABLE_USD, 5)] {
            let path = crate::examples::path(file);
            if !std::path::Path::new(&path).exists() { continue; }
            let scene = load(std::path::Path::new(&path)).unwrap();
            assert_eq!(scene.meshes.len(), meshes, "{file}: {:?}", scene.notes);
        }
    }

    #[test]
    fn instances_share_their_prototype_meshes() {
        use crate::types::Placement;
        let path = write("instances.usda", &format!(r#"#usda 1.0
def Xform "asset" {{
    def Mesh "box" {{
{QUAD}    }}
}}
def Xform "a" (
    instanceable = true
    references = </asset>
)
{{
    double3 xformOp:translate = (10, 0, 0)
    uniform token[] xformOpOrder = ["xformOp:translate"]
}}
def Xform "b" (
    instanceable = true
    references = </asset>
)
{{
    double3 xformOp:translate = (20, 0, 0)
    uniform token[] xformOpOrder = ["xformOp:translate"]
}}
def PointInstancer "trees" {{
    rel prototypes = [</trees/protos/box>]
    int[] protoIndices = [0, 0, 0]
    point3f[] positions = [(0, 0, 0), (0, 5, 0), (0, 10, 0)]
    def Scope "protos" {{
        def Mesh "box" {{
{QUAD}        }}
    }}
}}
"#));
        let scene = load(&path).unwrap();
        let find = |p: &str| scene.meshes.iter().find(|m| m.path == p).unwrap_or_else(|| panic!("no {p}: {:?} {:?}", scene.meshes.iter().map(|m| &m.path).collect::<Vec<_>>(), scene.notes));
        // Two native instances: one shared mesh, each placed.
        let (a, b) = (find("/a/box"), find("/b/box"));
        assert!(Arc::ptr_eq(&a.mesh, &b.mesh));
        let x = |m: &UsdMesh| match &m.place { Placement::One(t) => t.w_axis.x, _ => panic!("not placed") };
        assert_eq!((x(a), x(b)), (10.0, 20.0));
        // The point instancer: one primitive holding three copies.
        let trees = find("/trees/protos/box");
        let ys: Vec<f32> = trees.place.matrices().iter().map(|m| m.w_axis.y).collect();
        assert_eq!(ys, vec![0.0, 5.0, 10.0]);
        // The asset itself, two instances and three points, two triangles each.
        assert_eq!(scene.triangles(), 2 * 6);
        // A copy made into a mesh lands where it is placed.
        let placed = crate::node_graph::nodes::place_mesh(&b.mesh, &b.place.matrices()[0]);
        assert!((bounds(&placed).0 - Vec3::new(20.0, 0.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn purposes_are_kept_and_render_knows_its_proxy() {
        use crate::types::Purpose;
        let path = write("purposes.usda", &format!(r#"#usda 1.0
def Xform "asset" {{
    def Scope "render" {{
        uniform token purpose = "render"
        def Mesh "hi" {{
{QUAD}        }}
    }}
    def Scope "proxy" {{
        uniform token purpose = "proxy"
        def Mesh "lo" {{
{QUAD}        }}
    }}
    def Mesh "helper" {{
        uniform token purpose = "guide"
{QUAD}    }}
}}
def Mesh "plain" {{
{QUAD}}}
"#));
        let scene = load(&path).unwrap();
        let purpose = |p: &str| scene.meshes.iter().find(|m| m.path == p).map(|m| m.purpose)
            .unwrap_or_else(|| panic!("no {p}: {:?}", scene.notes));
        assert_eq!(purpose("/asset/render/hi"), Purpose::Render { has_proxy: true });
        assert_eq!(purpose("/asset/proxy/lo"), Purpose::Proxy);
        assert_eq!(purpose("/asset/helper"), Purpose::Guide);
        assert_eq!(purpose("/plain"), Purpose::Default);

        // By default: the proxy stands in for the render mesh, guides are off.
        let h = crate::types::SceneHierarchy::default();
        let drawn: Vec<&str> = scene.meshes.iter().filter(|m| h.draws_purpose(m.purpose)).map(|m| m.path.as_str()).collect();
        assert_eq!(drawn, vec!["/asset/proxy/lo", "/plain"]);
    }

    #[test]
    fn curves_and_points_are_read_as_they_are_authored() {
        use crate::types::{CurveBasis, CurveWrap};
        let path = write("strands.usda", r#"#usda 1.0
def Xform "groom" {
    double3 xformOp:translate = (0, 10, 0)
    uniform token[] xformOpOrder = ["xformOp:translate"]
    def BasisCurves "hair" {
        uniform token type = "cubic"
        uniform token basis = "catmullRom"
        int[] curveVertexCounts = [4, 4]
        point3f[] points = [(0, 0, 0), (0, 1, 0), (0, 2, 0), (0, 3, 0), (1, 0, 0), (1, 1, 0), (1, 2, 0), (1, 3, 0)]
        float[] widths = [0.1]
    }
    def Points "dust" {
        point3f[] points = [(5, 0, 0), (6, 0, 0)]
        float[] widths = [0.5, 0.25]
    }
}
"#);
        let scene = load(&path).unwrap();
        let find = |p: &str| scene.meshes.iter().find(|m| m.path == p).unwrap_or_else(|| panic!("no {p}: {:?}", scene.notes));
        let hair = &world(find("/groom/hair"));
        assert_eq!(hair.curve_counts, vec![4, 4]);
        assert_eq!((hair.curve_basis, hair.curve_wrap), (CurveBasis::CatmullRom, CurveWrap::Nonperiodic));
        // In world space, like meshes.
        assert_eq!(hair.curve_points[1], [0.0, 11.0, 0.0]);
        let dust = &world(find("/groom/dust"));
        assert_eq!(dust.points, vec![[5.0, 10.0, 0.0], [6.0, 10.0, 0.0]]);
        assert_eq!(dust.widths, vec![0.5, 0.25]);
        // Listed in the hierarchy as geometry, like meshes.
        assert!(scene.tree.nodes.iter().any(|n| n.path == "/groom/hair" && n.type_name == "BasisCurves"));
    }

    #[test]
    fn geometry_stays_local_with_every_primvar() {
        use crate::core::geo::{Context, Role, ST};
        let path = write("primvars.usda", r#"#usda 1.0
(
    upAxis = "Z"
)
def Xform "set" {
    double3 xformOp:translate = (0, 0, 5)
    uniform token[] xformOpOrder = ["xformOp:translate"]
    def Mesh "card" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    )
    {
        int[] faceVertexCounts = [4]
        int[] faceVertexIndices = [0, 1, 2, 3]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]
        uniform token orientation = "leftHanded"
        texCoord2f[] primvars:st = [(0, 0), (1, 0), (1, 1), (0, 1)] (
            interpolation = "faceVarying"
        )
        int[] primvars:st:indices = [0, 1, 2, 3]
        color3f[] primvars:displayColor = [(1, 0, 0)] (
            interpolation = "constant"
        )
        float[] primvars:heat = [0.5] (
            interpolation = "uniform"
        )
    }
}
"#);
        let scene = load(&path).unwrap();
        let card = &scene.meshes[0];
        let geo = card.geo.as_ref().expect("loaded as geometry");
        // Points as authored: the transform and the Z-up correction are in the placement.
        assert_eq!(geo.points()[2], [1.0, 1.0, 0.0]);
        let (lo, _) = bounds(&world(card));
        // Z up: authored +Y becomes -Z, authored Z (the set's 5) becomes Y.
        assert!((lo - Vec3::new(0.0, 5.0, -1.0)).length() < 1e-4, "{lo}");
        assert_ne!(scene.tree.root, bevy::math::Mat4::IDENTITY);
        // Every primvar, in its context, with its indices and role.
        let st = geo.attr(Context::Corner, ST).expect("st per corner");
        assert_eq!((st.indices.as_ref().map(|i| i.len()), st.role), (Some(4), Role::TexCoord));
        assert_eq!(geo.attr(Context::Object, "displayColor").map(|a| a.role), Some(Role::Color));
        assert_eq!(geo.attr(Context::Primitive, "heat").and_then(|a| a.column.floats()), Some(&[0.5][..]));
        assert!(matches!(geo.topology, crate::core::geo::Topology::Mesh { left_handed: true, .. }));
        // The MeshData view still has its UVs per triangle corner.
        assert_eq!(card.mesh.uvs.len(), 6);
    }

    #[test]
    fn a_missing_file_is_an_error_and_loads_are_cached() {
        assert!(load_cached("/nowhere/at/all.usda").is_err());
        let path = write("cached.usda", &format!("#usda 1.0\ndef Mesh \"m\" {{\n{QUAD}}}\n"));
        let a = load_cached(path.to_str().unwrap()).unwrap();
        let b = load_cached(path.to_str().unwrap()).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }

        #[test]
    fn references_and_variants_are_composed() {
        write("wheel.usda", &format!(r#"#usda 1.0
(
    defaultPrim = "wheel"
)
def Xform "wheel" (
    variantSets = "size"
    variants = {{ string size = "big" }}
)
{{
    variantSet "size" = {{
        "big" {{
            def Mesh "tyre" {{
                double3 xformOp:translate = (0, 5, 0)
                uniform token[] xformOpOrder = ["xformOp:translate"]
{QUAD}            }}
        }}
        "small" {{
            def Mesh "tyre" {{
{QUAD}            }}
        }}
    }}
}}
"#));
        let path = write("car.usda", r#"#usda 1.0
def Xform "car" {
    def "front" (
        references = @./wheel.usda@
    )
    {
    }
}
"#);
        let scene = load(&path).unwrap();
        // Composed, not the root layer alone.
        assert!(!scene.notes.iter().any(|n| n.contains("root layer only")), "{:?}", scene.notes);
        assert_eq!(scene.meshes.len(), 1, "{:?}", scene.notes);
        assert_eq!(scene.meshes[0].path, "/car/front/tyre");
        // The selected variant, "big", moved up by 5.
        assert!((bounds(&world(&scene.meshes[0])).0.y - 5.0).abs() < 1e-4);
        // The stage hierarchy holds the referenced prims.
        assert!(scene.tree.nodes.iter().any(|n| n.path == "/car/front/tyre" && n.depth == 3));
    }
}
