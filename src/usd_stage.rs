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
//! Meshes are baked into world space (Y-up metres), as the root-layer reader
//! did, so every node downstream works unchanged. Instanced meshes are not:
//! a native instance's mesh, and a point instancer's prototype mesh, is made
//! once in its own space and shared by every copy, each copy placed by a
//! matrix (`Placement`). A native instance gives one packed primitive per mesh
//! under the instance's own path; a point instancer gives one per prototype
//! mesh, holding all its points.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use bevy::math::{DMat4, DQuat, DVec3, Mat4};
use xms_scene as rs;

use crate::types::{MeshData, Placement};
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
        walk.prim(child, child.as_str().to_string(), root, None, 1, false, false);
    }
    let Walk { nodes, counts, .. } = walk;
    scene.tree = Arc::new(StageTree { nodes });
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
    curves:  usize,
    points:  usize,
    gprims:  usize,
    subsets: usize,
    hidden:  usize,
}

impl Counts {
    fn notes(&self) -> Vec<String> {
        let mut notes = vec![];
        let mut say = |n: usize, text: &str| if n > 0 { notes.push(format!("{n} {text}")) };
        say(self.nested, "point instancers inside the prototypes of another point instancer are not expanded");
        say(self.curves, "curves are not read into packed primitives yet");
        say(self.points, "point clouds are not read into packed primitives yet");
        say(self.gprims, "implicit shapes (sphere, cube, cylinder…) are not read yet");
        say(self.subsets, "meshes have face subsets (per-face materials): the mesh's own material is used");
        say(self.hidden, "invisible or guide/proxy meshes are listed but not drawn");
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
    shared:  HashMap<String, Option<Arc<MeshData>>>,
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
    fn prim(&mut self, src: &rs::Path, shown: String, parent: DMat4, binding: Option<String>, depth: usize, hidden: bool, under_instance: bool) {
        // The scene outlives the walk: borrow it apart from `self`.
        let sc = self.sc;
        let Some(prim) = sc.get(src) else { return };
        let t = self.t;
        let type_name = prim.type_name.clone();
        let key = if type_name.is_empty() { "(untyped)".to_string() } else { type_name.clone() };
        *self.scene.prim_counts.entry(key).or_default() += 1;

        let hidden = hidden || !prim.visible_at(t) || matches!(prim.purpose, rs::Purpose::Guide | rs::Purpose::Proxy);
        let name = shown.rsplit('/').next().unwrap_or("").to_string();
        self.nodes.push(StageNode { path: shown.clone(), name, type_name: type_name.clone(), depth, hidden });

        let local = mat(&prim.local_xform_at(t));
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
                    self.prim(child, child_shown, world, binding.clone(), depth + 1, hidden, true);
                }
                return;
            }
            rs::PrimKind::Mesh(m) => {
                if self.listing > 0 {
                    // A point instancer's prototype: drawn through the instancer.
                } else if hidden {
                    self.counts.hidden += 1;
                } else if under_instance {
                    // Shared with every other instance of the same prototype.
                    if let Some(shared) = self.shared_mesh(src, m) {
                        if !m.subsets.is_empty() { self.counts.subsets += 1; }
                        self.scene.meshes.push(UsdMesh { path: shown.clone(), mesh: shared, material: binding.clone(), place: Placement::One(world.as_mat4()) });
                    }
                } else if let Some(mesh) = mesh(m, t, &world) {
                    if !m.subsets.is_empty() { self.counts.subsets += 1; }
                    self.scene.meshes.push(UsdMesh { path: shown.clone(), mesh: Arc::new(mesh), material: binding.clone(), place: Placement::InPlace });
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
                    self.point_instancer(src, &shown, i, world, binding.clone());
                }
                // Its prototypes usually sit below it: listed, not drawn as they are.
                self.listing += 1;
                for child in sc.children(src) {
                    let child_shown = format!("{shown}/{}", child.name());
                    self.prim(child, child_shown, world, binding.clone(), depth + 1, hidden, under_instance);
                }
                self.listing -= 1;
                return;
            }
            rs::PrimKind::Curves(_) => self.counts.curves += 1,
            rs::PrimKind::Points(_) => self.counts.points += 1,
            rs::PrimKind::Gprim(_) => self.counts.gprims += 1,
            _ => {
                if type_name == "Skeleton" && !under_instance {
                    self.scene.skeletons.push(UsdSkeleton { path: shown.clone(), joints: vec![] });
                }
            }
        }
        for child in sc.children(src) {
            let child_shown = format!("{shown}/{}", child.name());
            self.prim(child, child_shown, world, binding.clone(), depth + 1, hidden, under_instance);
        }
    }

    /// A mesh in its own space, made once and shared by every copy.
    fn shared_mesh(&mut self, src: &rs::Path, m: &rs::Mesh) -> Option<Arc<MeshData>> {
        let t = self.t;
        self.shared.entry(src.as_str().to_string()).or_insert_with(|| mesh(m, t, &DMat4::IDENTITY).map(Arc::new)).clone()
    }

    /// A point instancer: one packed primitive per mesh of each prototype,
    /// placed at every point that uses that prototype.
    fn point_instancer(&mut self, src: &rs::Path, shown: &str, i: &rs::Instancer, world: DMat4, binding: Option<String>) {
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
            self.gather(proto, proto_shown, DMat4::IDENTITY, binding.clone(), &mut found);
            for (mesh_src, mesh_shown, inside, material) in found {
                let Some(rs::PrimKind::Mesh(m)) = sc.get(&mesh_src).map(|p| &p.kind) else { continue };
                let Some(shared) = self.shared_mesh(&mesh_src, m) else { continue };
                if !m.subsets.is_empty() { self.counts.subsets += 1; }
                let place: Arc<[Mat4]> = at.iter().map(|p| (*p * inside).as_mat4()).collect();
                self.scene.meshes.push(UsdMesh { path: mesh_shown, mesh: shared, material, place: Placement::Many(place) });
            }
        }
    }

    /// The meshes of a prototype, with their transform inside it (its own
    /// root's included), their path as listed, and their material.
    fn gather(&mut self, src: &rs::Path, shown: String, parent: DMat4, binding: Option<String>, out: &mut Vec<(rs::Path, String, DMat4, Option<String>)>) {
        let (sc, t) = (self.sc, self.t);
        let Some(prim) = sc.get(src) else { return };
        if !prim.visible_at(t) || matches!(prim.purpose, rs::Purpose::Guide | rs::Purpose::Proxy) { return; }
        let local = mat(&prim.local_xform_at(t));
        let inside = if prim.reset_xform_stack { local } else { parent * local };
        let binding = prim.material_binding.as_ref().map(|p| p.as_str().to_string()).or(binding);
        let below: &[rs::Path] = match &prim.kind {
            rs::PrimKind::Mesh(_) => { out.push((src.clone(), shown.clone(), inside, binding.clone())); sc.children(src) }
            rs::PrimKind::Instance { prototype } => sc.children(prototype),
            rs::PrimKind::Instancer(_) => { self.counts.nested += 1; return; }
            rs::PrimKind::Material(_) => return,
            _ => sc.children(src),
        };
        for c in below {
            self.gather(c, format!("{shown}/{}", c.name()), inside, binding.clone(), out);
        }
    }
}

/// The scene layer's row-vector matrix as a column-vector one: USD's rows are
/// the columns here, so the numbers are taken as they come.
fn mat(m: &rs::Mat4d) -> DMat4 {
    DMat4::from_cols_array_2d(&m.0)
}

// ── Meshes ───────────────────────────────────────────────────────────────────

/// Names tried for the texture coordinates, before any other 2D primvar.
const UV_NAMES: [&str; 5] = ["st", "st0", "UVMap", "map1", "uv"];

/// Where a UV value is looked up.
#[derive(Clone, Copy, PartialEq)]
enum Rate { Constant, Uniform, Vertex, Corner }

fn mesh(m: &rs::Mesh, t: f64, world: &DMat4) -> Option<MeshData> {
    let points = m.points.at(t)?;
    let (counts, indices) = (&m.face_vertex_counts, &m.face_vertex_indices);
    if points.len() < 3 || indices.is_empty() { return None; }
    let vertices: Vec<[f32; 3]> = points.iter()
        .map(|p| world.transform_point3(DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64)).as_vec3().to_array())
        .collect();
    // A mirrored transform or a left-handed mesh turns the faces inside out.
    let flip = m.left_handed != (world.determinant() < 0.0);

    // Texture coordinates: the usual names first, then any 2D primvar.
    let uv_pv = UV_NAMES.iter().find_map(|n| m.primvars.iter().find(|p| p.name == *n))
        .or_else(|| m.primvars.iter().find(|p| matches!(p.values.at(t), Some(rs::PrimvarValues::Float2(_)))));
    let uv = uv_pv.and_then(|pv| {
        let values: Vec<[f32; 2]> = match pv.values.at(t)? {
            rs::PrimvarValues::Float2(v) => v,
            _ => return None,
        };
        let count = pv.indices.as_ref().map(|i| i.len()).unwrap_or(values.len());
        let rate = match pv.interpolation {
            Some(rs::Interpolation::FaceVarying) => Rate::Corner,
            Some(rs::Interpolation::Vertex) | Some(rs::Interpolation::Varying) => Rate::Vertex,
            Some(rs::Interpolation::Uniform) => Rate::Uniform,
            Some(rs::Interpolation::Constant) => Rate::Constant,
            None if count == indices.len() && count != vertices.len() => Rate::Corner,
            None => Rate::Vertex,
        };
        Some((values, pv.indices.clone(), rate))
    });
    let uv_at = |corner: usize, vertex: usize, face: usize| -> [f32; 2] {
        let Some((values, index, rate)) = &uv else { return [0.0; 2] };
        let slot = match rate { Rate::Corner => corner, Rate::Vertex => vertex, Rate::Uniform => face, Rate::Constant => 0 };
        let i = match index { Some(ix) => ix.get(slot).copied().unwrap_or(0) as usize, None => slot };
        values.get(i).copied().unwrap_or([0.0; 2])
    };

    let mut tris: Vec<u32> = Vec::with_capacity(indices.len() * 2);
    let mut uvs: Vec<[f32; 2]> = if uv.is_some() { Vec::with_capacity(indices.len() * 2) } else { vec![] };
    let mut polys: Vec<Vec<u32>> = Vec::with_capacity(counts.len());
    let mut at = 0usize;
    for (face, n) in counts.iter().enumerate() {
        let n = *n as usize;
        if at + n > indices.len() { break; }
        if n >= 3 && indices[at..at + n].iter().all(|i| (*i as usize) < vertices.len()) {
            // Corners of this face, in the winding the mesh ends up with.
            let corners: Vec<usize> = if flip { (0..n).rev().map(|k| at + k).collect() } else { (0..n).map(|k| at + k).collect() };
            polys.push(corners.iter().map(|c| indices[*c]).collect());
            for k in 1..n - 1 {
                for c in [corners[0], corners[k], corners[k + 1]] {
                    tris.push(indices[c]);
                    if uv.is_some() { uvs.push(uv_at(c, indices[c] as usize, face)); }
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
