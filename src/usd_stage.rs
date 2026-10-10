//! Composed USD stages, read through `xms-usd`.
//!
//! `xms-usd` opens the stage with `openusd`, which resolves composition
//! (sublayers, references, payloads, inherits, specializes, variants, in text
//! and binary layers), and translates it into a scene layer (`xms-scene`): raw,
//! USD-shaped prims keyed by path. This module turns that scene layer into the
//! `UsdScene` the rest of the program already knows: meshes as packed
//! primitives, materials, cameras, lights, and the stage hierarchy for the
//! scene explorer.
//!
//! Geometry stays in its own space, as a `Geo` with its topology and every
//! primvar (normals, UVs with their indices and interpolation, displayColor,
//! custom ones). Each packed primitive is placed by its world transform,
//! which includes the stage's up-axis and unit correction (also kept on the
//! stage tree as `root`, so writing can take it off again). Instanced
//! geometry is shared: a native instance's mesh, and a point instancer's
//! prototype mesh, is made once and placed by every copy. A native instance
//! gives one packed primitive per mesh under the instance's own path; a
//! point instancer gives one per prototype mesh, holding all its points.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use bevy::math::{DMat4, DQuat, DVec3, Mat4};
use xms_scene as rs;

use crate::core::geo::{self, Attr, Column, Context, Geo, Kind, Role, Subdiv, Topology};
use crate::types::{MeshData, Placement, Purpose};
use crate::usd_scene::{StageNode, StageTree, UsdCamera, UsdMaterial, UsdMesh, UsdScene, UsdSkeleton};

/// Read a composed stage. `layer` is the root layer (a `.usdz` is unpacked
/// beforehand, so its textures are files on disk).
pub fn read(layer: &Path) -> Result<UsdScene, String> {
    let translated = xms_usd::translate(layer, Default::default())?;
    let sc = &translated.scene;
    let info = &sc.info;

    let mut scene = UsdScene {
        up_axis: if info.z_up { "Z".into() } else { "Y".into() },
        meters_per_unit: info.meters_per_unit,
        // The scene layer does not carry timeCodesPerSecond yet.
        time: info.time_range.map(|(start, end)| (start, end, 24.0)),
        ..Default::default()
    };

    // To Y-up, and to metres when the stage says what its unit is.
    let mut root = DMat4::IDENTITY;
    if info.z_up { root = DMat4::from_rotation_x(-std::f64::consts::FRAC_PI_2); }
    if let Some(unit) = info.meters_per_unit {
        if unit > 0.0 && (unit - 1.0).abs() > 1e-9 { root *= DMat4::from_scale(DVec3::splat(unit)); }
    }

    let mut walk = Walk { sc, t: sc.time().time, root, scene: &mut scene, nodes: vec![], counts: Counts::default(), shared: HashMap::new(), listing: 0 };
    for child in sc.children(&rs::Path::root()) {
        // Native-instance prototypes are reached through their instances.
        if child.name().starts_with("__Prototype") { continue; }
        walk.prim(child, child.as_str().to_string(), root, None, Purpose::Default, 1, false, false);
    }
    let Walk { nodes, counts, .. } = walk;
    scene.tree = Arc::new(StageTree {
        nodes, root: root.as_mat4(), source: layer.to_path_buf(),
        up_axis: scene.up_axis.clone(), meters_per_unit: scene.meters_per_unit,
    });
    scene.notes = counts.notes();
    // The translator's own notes, minus timings and empty reports.
    scene.notes.extend(translated.warnings.into_iter().filter(|w| {
        !w.starts_with("Scene-layer translation:") && !w.starts_with("Prototypes in the scene layer: 0 ")
    }));
    Ok(scene)
}

/// What the stage holds that is not turned into packed primitives yet.
#[derive(Default)]
struct Counts {
    nested:  usize,
    invalid: usize,
    gprims:  usize,
    subsets: usize,
    hidden:  usize,
}

impl Counts {
    fn notes(&self) -> Vec<String> {
        let mut notes = vec![];
        let mut say = |n: usize, text: &str| if n > 0 { notes.push(format!("{n} {text}")) };
        say(self.invalid, "meshes, curves or points have inconsistent topology or primvars and are left out");
        say(self.nested, "point instancers inside the prototypes of another point instancer are not expanded");
        say(self.gprims, "implicit shapes (sphere, cube, cylinder…) are not read yet");
        say(self.subsets, "meshes have face subsets (per-face materials): the mesh's own material is used");
        say(self.hidden, "invisible meshes are listed but not drawn");
        notes
    }
}

struct Walk<'a> {
    sc:      &'a rs::Scene,
    t:       f64,
    root:    DMat4,
    scene:   &'a mut UsdScene,
    nodes:   Vec<StageNode>,
    counts:  Counts,
    /// Instanced meshes in their own space, by their path in the scene
    /// layer: made once, shared by every copy.
    shared:  HashMap<String, Option<(Arc<Geo>, Arc<MeshData>)>>,
    /// Inside a point instancer: prims are listed, its prototypes are drawn
    /// through the instancer instead.
    listing: usize,
}

impl Walk<'_> {
    /// One prim and what is below it. `src` is where the scene layer holds it
    /// (inside a prototype, for an instance proxy); `shown` is its path as the
    /// user sees it. `under_instance`: inside an expanded instance, where
    /// materials and cameras are not collected again.
    #[allow(clippy::too_many_arguments)]
    fn prim(&mut self, src: &rs::Path, shown: String, parent: DMat4, binding: Option<String>, purpose: Purpose, depth: usize, hidden: bool, under_instance: bool) {
        // The scene outlives the walk: borrow it apart from `self`.
        let sc = self.sc;
        let Some(prim) = sc.get(src) else { return };
        let t = self.t;
        let type_name = prim.type_name.clone();
        let key = if type_name.is_empty() { "(untyped)".to_string() } else { type_name.clone() };
        *self.scene.prim_counts.entry(key).or_default() += 1;

        let hidden = hidden || !prim.visible_at(t);
        let purpose = purpose_of(sc, src, &prim.purpose, purpose);
        let name = shown.rsplit('/').next().unwrap_or("").to_string();
        let local = mat(&prim.local_xform_at(t));
        self.nodes.push(StageNode {
            path: shown.clone(), name, type_name: type_name.clone(), depth, hidden, local,
            reset_xform: prim.reset_xform_stack, proxy: under_instance, prototype: self.listing > 0,
        });

        let world = if prim.reset_xform_stack { self.root * local } else { parent * local };
        let binding = prim.material_binding.as_ref().map(|p| p.as_str().to_string()).or(binding);

        match &prim.kind {
            rs::PrimKind::Material(m) => {
                // Its shaders are held inside it, not as prims of their own.
                if !m.nodes.is_empty() { *self.scene.prim_counts.entry("Shader".into()).or_default() += m.nodes.len(); }
                if !under_instance { self.scene.materials.push(material(src.as_str(), m)); }
                return;
            }
            rs::PrimKind::Instance { prototype } => {
                for child in sc.children(prototype) {
                    let child_shown = format!("{shown}/{}", child.name());
                    self.prim(child, child_shown, world, binding.clone(), purpose, depth + 1, hidden, true);
                }
                return;
            }
            rs::PrimKind::Mesh(_) | rs::PrimKind::Curves(_) | rs::PrimKind::Points(_) => {
                if let rs::PrimKind::Mesh(m) = &prim.kind {
                    if !m.subsets.is_empty() && !hidden && self.listing == 0 { self.counts.subsets += 1; }
                }
                if self.listing > 0 {
                    // A point instancer's prototype: drawn through the instancer.
                } else if hidden {
                    self.counts.hidden += 1;
                } else {
                    // In its own space, placed by its world transform (root
                    // correction included). Under an instance it is shared with
                    // every other instance of the same prototype.
                    let made = if under_instance {
                        self.shared_geometry(src, &prim.kind)
                    } else {
                        geometry(&prim.kind, t).map(shared)
                    };
                    match made {
                        Some((geo, mesh)) => self.scene.meshes.push(UsdMesh {
                            path: shown.clone(), mesh, geo: Some(geo), material: binding.clone(),
                            place: Placement::One(world.as_mat4()), purpose,
                        }),
                        None => self.counts.invalid += 1,
                    }
                }
            }
            rs::PrimKind::Camera(c) if !under_instance => self.scene.cameras.push(UsdCamera {
                path:         shown.clone(),
                projection:   if c.projection_orthographic { "orthographic".into() } else { "perspective".into() },
                focal_length: c.focal_length.at(t).unwrap_or(50.0) as f32,
                aperture:     [c.horizontal_aperture as f32, c.vertical_aperture as f32],
                clip:         [c.clipping_range[0] as f32, c.clipping_range[1] as f32],
                position:     world.transform_point3(DVec3::ZERO).as_vec3().to_array(),
            }),
            rs::PrimKind::Light(_) if !under_instance => self.scene.lights.push((shown.clone(), type_name.clone())),
            rs::PrimKind::Instancer(i) => {
                if self.listing > 0 {
                    self.counts.nested += 1;
                } else if !hidden {
                    self.point_instancer(src, &shown, i, world, binding.clone(), purpose);
                }
                // Its prototypes usually sit below it: listed, not drawn as they are.
                self.listing += 1;
                for child in sc.children(src) {
                    let child_shown = format!("{shown}/{}", child.name());
                    self.prim(child, child_shown, world, binding.clone(), purpose, depth + 1, hidden, under_instance);
                }
                self.listing -= 1;
                return;
            }
            rs::PrimKind::Gprim(_) => self.counts.gprims += 1,
            _ => {
                if type_name == "Skeleton" && !under_instance {
                    self.scene.skeletons.push(UsdSkeleton { path: shown.clone(), joints: vec![] });
                }
            }
        }
        for child in sc.children(src) {
            let child_shown = format!("{shown}/{}", child.name());
            self.prim(child, child_shown, world, binding.clone(), purpose, depth + 1, hidden, under_instance);
        }
    }

    /// A mesh, curves or points in their own space, made once and shared by
    /// every copy.
    fn shared_geometry(&mut self, src: &rs::Path, kind: &rs::PrimKind) -> Option<(Arc<Geo>, Arc<MeshData>)> {
        let t = self.t;
        self.shared.entry(src.as_str().to_string()).or_insert_with(|| geometry(kind, t).map(shared)).clone()
    }

    /// A point instancer: one packed primitive per mesh of each prototype,
    /// placed at every point that uses that prototype.
    fn point_instancer(&mut self, src: &rs::Path, shown: &str, i: &rs::Instancer, world: DMat4, binding: Option<String>, purpose: Purpose) {
        let (sc, t) = (self.sc, self.t);
        let positions = i.positions.at(t).unwrap_or_default();
        let scales = i.scales.as_ref().and_then(|s| s.at(t)).unwrap_or_default();
        let orients = i.orientations.as_ref().and_then(|o| o.at(t)).unwrap_or_default();
        let invisible: HashSet<i64> = i.invisible_ids.iter().copied().collect();
        // Each point's placement, by prototype: scaled, then turned, then moved.
        let mut points: Vec<Vec<DMat4>> = vec![vec![]; i.prototypes.len()];
        for (k, &proto) in i.proto_indices.iter().enumerate() {
            let id = i.ids.as_ref().and_then(|ids| ids.get(k).copied()).unwrap_or(k as i64);
            if invisible.contains(&id) { continue; }
            let Some(list) = points.get_mut(proto as usize) else { continue };
            let p = positions.get(k).copied().unwrap_or([0.0; 3]);
            let s = scales.get(k).copied().unwrap_or([1.0; 3]);
            // Stored (i, j, k, real).
            let q = orients.get(k).map(|q| DQuat::from_xyzw(q[0] as f64, q[1] as f64, q[2] as f64, q[3] as f64).normalize()).unwrap_or(DQuat::IDENTITY);
            list.push(world * DMat4::from_scale_rotation_translation(
                DVec3::new(s[0] as f64, s[1] as f64, s[2] as f64), q, DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64)));
        }
        for (proto, at) in i.prototypes.iter().zip(&points) {
            if at.is_empty() { continue; }
            // Listed under the instancer when it lives there, as the hierarchy shows it.
            let proto_shown = match proto.as_str().strip_prefix(src.as_str()).filter(|r| r.starts_with('/')) {
                Some(rest) => format!("{shown}{rest}"),
                None => format!("{shown}/{}", proto.name()),
            };
            let mut found = vec![];
            self.gather(proto, proto_shown, DMat4::IDENTITY, binding.clone(), purpose, &mut found);
            for (mesh_src, mesh_shown, inside, material, purpose) in found {
                let Some(kind) = sc.get(&mesh_src).map(|p| &p.kind) else { continue };
                let Some((geo, shared)) = self.shared_geometry(&mesh_src, kind) else { self.counts.invalid += 1; continue };
                if let rs::PrimKind::Mesh(m) = kind { if !m.subsets.is_empty() { self.counts.subsets += 1; } }
                let place: Arc<[Mat4]> = at.iter().map(|p| (*p * inside).as_mat4()).collect();
                self.scene.meshes.push(UsdMesh { path: mesh_shown, mesh: shared, geo: Some(geo), material, place: Placement::Many(place), purpose });
            }
        }
    }

    /// The meshes of a prototype, with their transform inside it (its own
    /// root's included), their path as listed, and their material.
    #[allow(clippy::type_complexity)]
    fn gather(&mut self, src: &rs::Path, shown: String, parent: DMat4, binding: Option<String>, purpose: Purpose, out: &mut Vec<(rs::Path, String, DMat4, Option<String>, Purpose)>) {
        let (sc, t) = (self.sc, self.t);
        let Some(prim) = sc.get(src) else { return };
        if !prim.visible_at(t) { return; }
        let purpose = purpose_of(sc, src, &prim.purpose, purpose);
        let local = mat(&prim.local_xform_at(t));
        let inside = if prim.reset_xform_stack { local } else { parent * local };
        let binding = prim.material_binding.as_ref().map(|p| p.as_str().to_string()).or(binding);
        let below: &[rs::Path] = match &prim.kind {
            rs::PrimKind::Mesh(_) | rs::PrimKind::Curves(_) | rs::PrimKind::Points(_) => { out.push((src.clone(), shown.clone(), inside, binding.clone(), purpose)); sc.children(src) }
            rs::PrimKind::Instance { prototype } => sc.children(prototype),
            rs::PrimKind::Instancer(_) => { self.counts.nested += 1; return; }
            rs::PrimKind::Material(_) => return,
            _ => sc.children(src),
        };
        for c in below {
            self.gather(c, format!("{shown}/{}", c.name()), inside, binding.clone(), purpose, out);
        }
    }
}

/// A prim's purpose: its own when authored, else its parent's. Render
/// geometry notes whether its asset also carries proxy geometry: a sibling of
/// the prim where render was set that is itself set to proxy (the usual
/// `render` / `proxy` pair under one asset).
fn purpose_of(sc: &rs::Scene, src: &rs::Path, own: &rs::Purpose, inherited: Purpose) -> Purpose {
    match own {
        rs::Purpose::Render => Purpose::Render {
            has_proxy: src.parent().is_some_and(|parent| {
                sc.children(&parent).iter().any(|c| sc.get(c).is_some_and(|p| matches!(p.purpose, rs::Purpose::Proxy)))
            }),
        },
        rs::Purpose::Proxy => Purpose::Proxy,
        rs::Purpose::Guide => Purpose::Guide,
        _ => inherited,
    }
}

/// The scene layer's row-vector matrix as a column-vector one: USD's rows are
/// the columns here, so the numbers are taken as they come.
fn mat(m: &rs::Mat4d) -> DMat4 {
    DMat4::from_cols_array_2d(&m.0)
}

// ── Geometry ─────────────────────────────────────────────────────────────────

/// A mesh, curves or points prim as geometry in its own space: topology and
/// every primvar, as USD has them. Placed in the scene by its packed
/// primitive's `Placement`, not by moving its points.
fn geometry(kind: &rs::PrimKind, t: f64) -> Option<Geo> {
    let geo = match kind {
        rs::PrimKind::Mesh(m) => mesh_geo(m, t)?,
        rs::PrimKind::Curves(c) => curves_geo(c, t)?,
        rs::PrimKind::Points(p) => points_geo(p, t)?,
        _ => return None,
    };
    geo.validate().ok()?;
    Some(geo)
}

/// The geometry and its `MeshData` view, made once per prim (or prototype).
fn shared(geo: Geo) -> (Arc<Geo>, Arc<MeshData>) {
    let view = Arc::new(geo.to_mesh());
    (Arc::new(geo), view)
}

fn mesh_geo(m: &rs::Mesh, t: f64) -> Option<Geo> {
    let points = m.points.at(t)?;
    if points.len() < 3 || m.face_vertex_indices.is_empty() { return None; }
    let mut g = Geo::from_polygons(points, m.face_vertex_counts.clone(), m.face_vertex_indices.clone());
    if let Topology::Mesh { left_handed, subdiv, .. } = &mut g.topology {
        *left_handed = m.left_handed;
        *subdiv = match m.subdivision.scheme.as_deref() {
            Some("none") => Subdiv::None,
            Some("loop") => Subdiv::Loop,
            Some("bilinear") => Subdiv::Bilinear,
            _ => Subdiv::CatmullClark,
        };
    }
    if let Some(n) = &m.normals { add_primvar(&mut g, n, t); }
    for pv in &m.primvars { add_primvar(&mut g, pv, t); }
    Some(g)
}

fn curves_geo(c: &rs::Curves, t: f64) -> Option<Geo> {
    use crate::types::{CurveBasis, CurveWrap};
    let points = c.points.at(t)?;
    let total: usize = c.curve_vertex_counts.iter().map(|&n| n as usize).sum();
    if points.is_empty() || c.curve_vertex_counts.is_empty() || total > points.len() { return None; }
    let basis = if c.curve_type == "linear" { CurveBasis::Linear } else {
        match c.basis.as_str() { "bspline" => CurveBasis::Bspline, "catmullRom" => CurveBasis::CatmullRom, _ => CurveBasis::Bezier }
    };
    let wrap = match c.wrap.as_str() { "periodic" => CurveWrap::Periodic, "pinned" => CurveWrap::Pinned, _ => CurveWrap::Nonperiodic };
    let mut g = Geo::from_points(points);
    g.topology = Topology::curves(c.curve_vertex_counts.clone(), basis, wrap);
    for pv in c.widths.iter().chain(&c.normals).chain(&c.primvars) { add_primvar(&mut g, pv, t); }
    Some(g)
}

fn points_geo(p: &rs::Points, t: f64) -> Option<Geo> {
    let points = p.points.at(t)?;
    if points.is_empty() { return None; }
    let mut g = Geo::from_points(points);
    for pv in p.widths.iter().chain(&p.primvars) { add_primvar(&mut g, pv, t); }
    Some(g)
}

/// A primvar as an attribute, in the context of its interpolation. Without
/// one, the context is the one whose element count it matches (corners
/// first, as faceVarying UVs are the common unauthored case). Primvars that
/// match no context, such as curve widths per segment end (`varying` on
/// curves), are left out: they stay in the file, unedited.
fn add_primvar(g: &mut Geo, pv: &rs::Primvar, t: f64) {
    let Some(values) = pv.values.at(t) else { return };
    let column = match values {
        rs::PrimvarValues::Float(v) => Column::Float(Arc::new(v)),
        rs::PrimvarValues::Float2(v) => Column::Vec2(Arc::new(v)),
        rs::PrimvarValues::Float3(v) => Column::Vec3(Arc::new(v)),
        rs::PrimvarValues::Float4(v) => Column::Vec4(Arc::new(v)),
        #[allow(unreachable_patterns)]
        _ => return,
    };
    let elements = pv.indices.as_ref().map_or(column.len(), |i| i.len());
    let by_count = || [Context::Corner, Context::Point, Context::Primitive, Context::Object]
        .into_iter()
        .find(|&c| g.count(c) == elements && elements > 0);
    let ctx = match pv.interpolation {
        Some(rs::Interpolation::Constant) => Some(Context::Object),
        Some(rs::Interpolation::Uniform) => Some(Context::Primitive),
        Some(rs::Interpolation::Vertex) => Some(Context::Point),
        Some(rs::Interpolation::FaceVarying) => Some(Context::Corner),
        // Per point for meshes and points; per segment end for curves.
        Some(rs::Interpolation::Varying) => Some(Context::Point).filter(|&c| g.count(c) == elements),
        None => by_count(),
    };
    let Some(ctx) = ctx.filter(|&c| g.count(c) == elements) else { return };
    let role = match (pv.name.as_str(), column.kind()) {
        (geo::NORMALS, _) => Role::Normal,
        ("displayColor", _) => Role::Color,
        (_, Kind::Vec2) if geo::UV_NAMES.contains(&pv.name.as_str()) => Role::TexCoord,
        _ => Role::None,
    };
    let attr = match &pv.indices {
        Some(ix) => Attr::indexed(column, ix.clone(), role),
        None => Attr::new(column, role),
    };
    g.set(ctx, &pv.name, attr);
}

// ── Materials ────────────────────────────────────────────────────────────────

/// A material as the viewport and the properties panel know it: the surface
/// shader's colour, roughness, metallic and opacity, and the texture behind
/// each of those inputs. OpenPBR and Standard Surface inputs are given the
/// names of the UsdPreviewSurface inputs they stand for.
fn material(path: &str, m: &rs::Material) -> UsdMaterial {
    let mut out = UsdMaterial { path: path.to_string(), ..Default::default() };
    let node = |p: &rs::Path| m.nodes.iter().find(|n| n.path == *p);
    let Some(shader) = m.surface.as_ref().and_then(|(p, _)| node(p)) else { return out };
    let roles: &[(&str, &str)] = if shader.id.contains("open_pbr_surface") {
        &[("base_color", "diffuseColor"), ("specular_roughness", "roughness"), ("base_metalness", "metallic"),
          ("geometry_opacity", "opacity"), ("emission_color", "emissiveColor")]
    } else if shader.id.contains("standard_surface") {
        &[("base_color", "diffuseColor"), ("specular_roughness", "roughness"), ("metalness", "metallic"),
          ("opacity", "opacity"), ("emission_color", "emissiveColor")]
    } else {
        &[("diffuseColor", "diffuseColor"), ("roughness", "roughness"), ("metallic", "metallic"),
          ("opacity", "opacity"), ("emissiveColor", "emissiveColor")]
    };
    for (input, role) in roles {
        let Some(i) = shader.inputs.iter().find(|i| i.name == *input) else { continue };
        match &i.value {
            rs::ShaderValue::Connection { node: src, .. } => {
                if let Some(file) = node(src).and_then(texture_file) { out.textures.push((role.to_string(), file)); }
            }
            v => match *role {
                "diffuseColor" => out.diffuse = color(v),
                "roughness" => out.roughness = number(v),
                "metallic" => out.metallic = number(v),
                // Standard Surface's opacity is a colour.
                "opacity" => out.opacity = number(v).or_else(|| color(v).map(|c| (c[0] + c[1] + c[2]) / 3.0)),
                _ => {}
            },
        }
    }
    out
}

/// The file a texture node reads (UsdUVTexture, MaterialX image nodes).
fn texture_file(n: &rs::ShaderNode) -> Option<String> {
    n.inputs.iter().find(|i| i.name == "file").and_then(|i| match &i.value {
        rs::ShaderValue::Asset(s) | rs::ShaderValue::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    })
}

fn number(v: &rs::ShaderValue) -> Option<f32> {
    match v {
        rs::ShaderValue::Float(f) => Some(*f),
        rs::ShaderValue::Int(i) => Some(*i as f32),
        _ => None,
    }
}

fn color(v: &rs::ShaderValue) -> Option<[f32; 3]> {
    match v {
        rs::ShaderValue::Vec3(c) => Some(*c),
        rs::ShaderValue::Vec4(c) => Some([c[0], c[1], c[2]]),
        rs::ShaderValue::Float(f) => Some([*f; 3]),
        _ => None,
    }
}
