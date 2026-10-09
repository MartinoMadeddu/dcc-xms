//! Import through the scene layer: walk an [`rray_scene::Scene`]
//! instead of the USD stage, emitting the same batches as [`Ctx::walk`]. The
//! readers here fill the same input structs as the USD readers and call the same
//! `emit_*` functions, so the two paths can only differ in what they read.
//!
//! Lights, the dome, cameras and render settings still read from the stage (at
//! the scene layer's transform), as do materials; they move in phase 2b-2.

use std::time::Instant;

use rray_render::geom::subdiv::Tags;
use rray_scene as rs;

use crate::curves::{CurveInput, CURVE_ST_NAMES};
use crate::gprims::{capsule, cone, cube, cylinder, gprim_axis, plane};
use crate::instancing::InstancerInput;
use crate::mesh::{normals_interp, primvar_interp, tags_from, texcoord_interp, MeshInput, MeshPrimvar, PRIMVAR_SKIP, TEXCOORD_NAMES};
use crate::*;

/// USD row-vector `Mat4d` → nalgebra column-vector matrix.
fn m4_from(m: &rs::Mat4d) -> M4 {
    M4::from_fn(|r, c| m.0[c][r])
}

fn interp_token(i: Option<rs::Interpolation>) -> Option<&'static str> {
    i.map(|i| match i {
        rs::Interpolation::Constant => "constant",
        rs::Interpolation::Uniform => "uniform",
        rs::Interpolation::Varying => "varying",
        rs::Interpolation::Vertex => "vertex",
        rs::Interpolation::FaceVarying => "faceVarying",
    })
}

/// Primvar values as (components, 4-wide values).
fn nv(v: &rs::PrimvarValues) -> (u8, Vec<[f32; 4]>) {
    match v {
        rs::PrimvarValues::Float(x) => (1, x.iter().map(|&a| [a, 0.0, 0.0, 0.0]).collect()),
        rs::PrimvarValues::Float2(x) => (2, x.iter().map(|a| [a[0], a[1], 0.0, 0.0]).collect()),
        rs::PrimvarValues::Float3(x) => (3, x.iter().map(|a| [a[0], a[1], a[2], 0.0]).collect()),
        rs::PrimvarValues::Float4(x) => (4, x.clone()),
        rs::PrimvarValues::Int(x) => (1, x.iter().map(|&a| [a as f32, 0.0, 0.0, 0.0]).collect()),
    }
}

/// A primvar's values at `t`, raw (as authored, indices not applied).
fn raw(pv: &rs::Primvar, t: f64) -> Option<(u8, Vec<[f32; 4]>)> {
    pv.values.at(t).map(|v| nv(&v))
}

/// A primvar's values at `t`, with its indices applied.
fn indexed(pv: &rs::Primvar, t: f64) -> Option<(u8, Vec<[f32; 4]>)> {
    let (dims, vals) = raw(pv, t)?;
    Some(match &pv.indices {
        Some(idx) => (dims, idx.iter().filter_map(|&i| vals.get(i as usize).copied()).collect()),
        None => (dims, vals),
    })
}

/// A primvar's values at `t` as floats, without indices (as [`raw`]); float arrays
/// are moved, not converted.
fn take_floats(pv: rs::Primvar, t: f64) -> Option<Vec<f32>> {
    Some(match pv.values.into_at(t)? {
        rs::PrimvarValues::Float(v) => v,
        other => nv(&other).1.into_iter().map(|x| x[0]).collect(),
    })
}

/// A 3-component primvar's values at `t`, without indices (as [`raw`] with 3 dims).
fn take_float3(pv: rs::Primvar, t: f64) -> Option<Vec<[f32; 3]>> {
    match pv.values.into_at(t)? {
        rs::PrimvarValues::Float3(v) => Some(v),
        other => {
            let (d, v) = nv(&other);
            (d == 3).then(|| v.into_iter().map(|x| [x[0], x[1], x[2]]).collect())
        }
    }
}

/// A 2-component primvar's values at `t` with its indices applied (as [`indexed`]).
fn take_float2_indexed(pv: rs::Primvar, t: f64) -> Option<Vec<[f32; 2]>> {
    let indices = pv.indices;
    let v = match pv.values.into_at(t)? {
        rs::PrimvarValues::Float2(v) => v,
        other => {
            let (d, v) = nv(&other);
            if d != 2 {
                return None;
            }
            v.into_iter().map(|x| [x[0], x[1]]).collect()
        }
    };
    Some(match indices {
        Some(idx) => idx.iter().filter_map(|&i| v.get(i as usize).copied()).collect(),
        None => v,
    })
}

/// Move the data of curve prims drawn exactly once (outside PointInstancers and native
/// prototypes, which can be walked more than once) out of the scene layer, so the walk
/// hands it to the curve builder without copying. The scene layer keeps the prims
/// (with empty data); it's freed after the walk anyway.
pub(crate) fn take_curve_payloads(sc: &mut rs::Scene) -> crate::HashMap<rs::Path, rs::Curves> {
    let mut out = crate::HashMap::default();
    let paths: Vec<rs::Path> = sc
        .traverse(&rs::Path::root())
        .into_iter()
        .filter(|p| matches!(sc.get(p).map(|q| &q.kind), Some(rs::PrimKind::Curves(_))))
        .filter(|p| !p.as_str().starts_with("/__Prototype"))
        .filter(|p| {
            let mut cur = p.parent();
            while let Some(a) = cur {
                if matches!(sc.get(&a).map(|q| &q.kind), Some(rs::PrimKind::Instancer(_))) {
                    return false;
                }
                cur = a.parent();
            }
            true
        })
        .collect();
    for p in paths {
        sc.edit(&p, rs::Dirty::NONE, |prim| {
            if let rs::PrimKind::Curves(c) = &mut prim.kind {
                out.insert(p.clone(), std::mem::take(&mut **c));
            }
        });
    }
    let _ = sc.take_changes();
    out
}

fn find<'a>(pvs: &'a [rs::Primvar], name: &str) -> Option<&'a rs::Primvar> {
    pvs.iter().find(|p| p.name == name)
}

impl<'s> Ctx<'s> {
    /// The scene-layer counterpart of [`Ctx::walk`].
    /// Points that change during the shutter (deformation blur): (the points at shutter
    /// open, normalized key times starting with 0, the points at each later key: every
    /// sample inside the shutter, and close). `None` when the shutter is closed, the
    /// points are static or don't change in it, or their count changes (topology
    /// changes can't be interpolated).
    pub(crate) fn point_keys(&self, pts: &rs::Sampled<Vec<rs::Vec3f>>, t: f64) -> Option<(Vec<[f32; 3]>, Vec<f32>, Vec<Vec<[f32; 3]>>)> {
        let (open, close) = (t + self.shutter.0, t + self.shutter.1);
        if close <= open || !pts.is_animated() {
            return None;
        }
        let mut times = vec![open];
        times.extend(pts.times().filter(|&s| s > open && s < close));
        times.push(close);
        let vals: Vec<Vec<[f32; 3]>> = times.iter().map(|&s| pts.at(s)).collect::<Option<_>>()?;
        let n = vals[0].len();
        if n == 0 || vals.iter().any(|v| v.len() != n) || vals.iter().all(|v| *v == vals[0]) {
            return None;
        }
        let span = close - open;
        let norm: Vec<f32> = times.iter().map(|&s| ((s - open) / span) as f32).collect();
        let mut it = vals.into_iter();
        let first = it.next()?;
        Some((first, norm, it.collect()))
    }

    /// A prim's world transforms across the shutter (the import's root rotation
    /// included), at the scene layer's sample times inside it plus its ends, with
    /// times normalized to [0, 1]; `None` when the shutter is closed or it doesn't move.
    pub(crate) fn motion_keys(&self, sc: &rs::Scene, path: &rs::Path, t: f64) -> Option<Arc<MotionKeys>> {
        let (open, close) = (t + self.shutter.0, t + self.shutter.1);
        if close <= open {
            return None;
        }
        let samples = sc.world_xform_samples(path, open, close);
        if samples.len() < 2 {
            return None;
        }
        let span = close - open;
        Some(Arc::new(MotionKeys {
            times: samples.iter().map(|(ts, _)| ((ts - open) / span) as f32).collect(),
            xforms: samples.iter().map(|(_, m)| to_rows(&(self.root * m4_from(m)))).collect(),
        }))
    }

    pub(crate) fn walk_scene(&mut self, sc: &rs::Scene, path: &rs::Path, parent: &M4, inh: &Inherit, parent_node: usize, out: &mut Emit) {
        if self.aborted {
            return;
        }
        let Some(prim) = sc.get(path) else { return };
        self.stats.prims += 1;
        let t = sc.time().time;
        let ty = prim.type_name.clone();
        let node = self.push_node(parent_node, path.name().to_string(), path.as_str().to_string(), ty.clone(), NodeFlags::default());

        // (Inactive and abstract prims, shaders and subsets aren't in the scene layer)
        if let rs::PrimKind::Material(_) = prim.kind {
            self.tree[node].flags.display_only = true;
            return;
        }
        if !prim.visible_at(t) {
            self.tree[node].flags.invisible = true;
            self.display_scene_subtree(sc, path, node);
            return;
        }
        if matches!(prim.purpose, rs::Purpose::Guide | rs::Purpose::Proxy) {
            self.tree[node].flags.non_render_purpose = true;
            self.display_scene_subtree(sc, path, node);
            return;
        }

        let local = m4_from(&prim.local_xform_at(t));
        let world = if prim.reset_xform_stack { self.root * local } else { parent * local };
        // Motion blur: this prim's world transforms across the shutter, when it moves
        // (prims inside prototypes move with their instances instead)
        self.cur_motion = if self.proto_depth == 0 { self.motion_keys(sc, path, t) } else { None };
        let mut inh = inh.clone();
        if let Some(m) = &prim.material_binding {
            inh.material = Some(m.as_str().to_string());
        }
        if let Some(o) = prim.opaque {
            inh.opaque = Some(o);
        }

        // Native instancing: the prototype's subtree is translated once
        if let rs::PrimKind::Instance { prototype } = &prim.kind {
            self.stats.instances += 1;
            self.tree[node].flags.instance = true;
            let data = self.prototype_data(prototype.as_str(), false);
            let map = self.graft(&data, node, NodeFlags { instance_proxy: true, ..NodeFlags::default() });
            self.emit_instance(&data, &world, &inh, Some(map.as_slice()), node, out);
            return;
        }

        self.cur_node = node;
        match &prim.kind {
            rs::PrimKind::Mesh(m) => self.mesh_scene(m, t, &world, &inh, out),
            rs::PrimKind::Gprim(g) => self.gprim_scene(prim, g, t, &world, &inh, out),
            rs::PrimKind::Points(p) => self.points_scene(p, t, &world, &inh, out),
            rs::PrimKind::Instancer(i) => {
                self.instancer_scene(i, t, &world, &inh, node, out);
                return; // prototypes live below; never draw them directly
            }
            rs::PrimKind::Curves(c) => {
                // Moved out of the scene layer beforehand when drawn once; else a copy
                let owned = self.curve_payloads.remove(path).unwrap_or_else(|| (**c).clone());
                self.curves_scene(prim, owned, t, &world, &inh, out)
            }
            // Lights, the dome, cameras and RenderSettings
            rs::PrimKind::Light(l) => {
                if self.proto_depth == 0 {
                    match &l.kind {
                        rs::LightKind::Dome { .. } => self.dome_scene(&prim.type_name, l, &world, t),
                        _ => self.light_scene(path.as_str(), l, &world, t),
                    }
                }
            }
            rs::PrimKind::Camera(c) => {
                if self.proto_depth == 0 {
                    // Moving during the shutter: the camera at open, plus its pose at close
                    let (open, close) = (t + self.shutter.0, t + self.shutter.1);
                    if close > open && sc.world_xform_samples(path, open, close).len() > 1 {
                        let w_open = self.root * m4_from(&sc.world_xform_at(path, open));
                        let w_close = self.root * m4_from(&sc.world_xform_at(path, close));
                        self.camera_scene(path.as_str(), c, &w_open, open, Some((&w_close, close)));
                    } else {
                        self.camera_scene(path.as_str(), c, &world, t, None);
                    }
                }
            }
            rs::PrimKind::RenderSettings(r) => self.render_settings_scene(path.as_str(), r),
            _ => match ty.as_str() {
                "BasisCurves" => *self.skipped.entry("BasisCurves (import disabled)".into()).or_default() += 1,
                "Skeleton" | "SkelAnimation" => {
                    if !self.skel_warned {
                        self.skel_warned = true;
                        self.warnings.push("UsdSkel skinning isn't applied; skinned meshes render in their authored pose".into());
                    }
                }
                "NurbsCurves" | "HermiteCurves" | "NurbsPatch" | "TetMesh" | "Volume" | "PortalLight" | "GeometryLight" => {
                    *self.skipped.entry(ty.clone()).or_default() += 1;
                }
                _ => {}
            },
        }

        for c in sc.children(path) {
            self.walk_scene(sc, c, &world, &inh, node, out);
        }
    }

    /// List a scene-layer subtree in the hierarchy without importing it.
    fn display_scene_subtree(&mut self, sc: &rs::Scene, path: &rs::Path, node: usize) {
        for c in sc.children(path) {
            if self.tree.len() >= MAX_NODES {
                self.nodes_capped = true;
                return;
            }
            let ty = sc.get(c).map(|p| p.type_name.clone()).unwrap_or_default();
            let n = self.push_node(node, c.name().to_string(), c.as_str().to_string(), ty, NodeFlags::default());
            self.tree[n].flags = self.tree[node].flags;
            self.tree[n].flags.display_only = true;
            self.display_scene_subtree(sc, c, n);
        }
    }

    /// Material key for a prim: its inherited binding, else its displayColor.
    fn surface_key_scene(&mut self, primvars: &[rs::Primvar], t: f64, inh: &Inherit) -> Option<String> {
        if let Some(m) = &inh.material {
            let m = m.clone();
            return Some(self.material_key(&m));
        }
        let (dims, vals) = raw(find(primvars, "displayColor")?, t)?;
        let c = vals.first().filter(|_| dims >= 3)?;
        Some(self.color_key([c[0], c[1], c[2]]))
    }

    fn mesh_scene(&mut self, m: &rs::Mesh, t: f64, world: &M4, inh: &Inherit, out: &mut Emit) {
        let t0 = Instant::now();
        self.stats.meshes += 1;
        // Deforming during the shutter (motion blur): the points at shutter open, plus
        // the later keys; otherwise the points at the frame, as always
        let (points, motion) = match self.point_keys(&m.points, t) {
            Some((open_points, times, keys)) => (open_points, Some((times, keys))),
            None => match m.points.at(t) {
                Some(p) => (p, None),
                None => return,
            },
        };
        let counts: Vec<i64> = m.face_vertex_counts.iter().map(|&c| c as i64).collect();
        let indices: Vec<i64> = m.face_vertex_indices.iter().map(|&i| i as i64).collect();
        if points.is_empty() || indices.is_empty() || counts.is_empty() {
            self.t_mesh += t0.elapsed();
            return;
        }
        let (n_points, n_fv, n_faces) = (points.len(), indices.len(), counts.len());
        let holes = m.hole_indices.iter().map(|&i| i as usize).collect();
        let authored = m.normals.as_ref().and_then(|pv| {
            let (dims, v) = indexed(pv, t)?;
            if dims != 3 {
                return None;
            }
            let vals: Vec<[f32; 3]> = v.iter().map(|x| [x[0], x[1], x[2]]).collect();
            let i = normals_interp(interp_token(pv.interpolation), vals.len(), n_points, n_fv, n_faces)?;
            Some((vals, i))
        });
        let uvs = TEXCOORD_NAMES.iter().find_map(|name| {
            let pv = find(&m.primvars, name)?;
            let (dims, v) = indexed(pv, t)?;
            if dims != 2 {
                return None;
            }
            let vals: Vec<[f32; 2]> = v.iter().map(|x| [x[0], x[1]]).collect();
            let i = texcoord_interp(interp_token(pv.interpolation), vals.len(), n_points, n_fv, n_faces)?;
            Some((vals, i))
        });
        let mut primvars = Vec::new();
        for pv in &m.primvars {
            if PRIMVAR_SKIP.contains(&pv.name.as_str()) {
                continue;
            }
            let Some((dims, values)) = indexed(pv, t) else { continue };
            if let Some(interp) = primvar_interp(interp_token(pv.interpolation), values.len(), n_points, n_fv, n_faces) {
                primvars.push(MeshPrimvar { name: pv.name.clone(), dims: dims.clamp(1, 4), values, interp });
            }
        }
        let sd = &m.subdivision;
        let tagged = sd.is_tagged();
        let as_i64 = |v: &[u32]| v.iter().map(|&x| x as i64).collect::<Vec<i64>>();
        let tags = if tagged {
            tags_from(&as_i64(&sd.crease_indices), &as_i64(&sd.crease_lengths), &sd.crease_sharpnesses, &as_i64(&sd.corner_indices), &sd.corner_sharpnesses)
        } else {
            Tags::default()
        };
        let base_key = self.surface_key_scene(&m.primvars, t, inh);
        let mut subsets = Vec::new();
        for s in &m.subsets {
            let Some(mat) = &s.material_binding else { continue };
            if s.faces.is_empty() {
                continue;
            }
            let key = self.material_key(mat.as_str());
            subsets.push((s.faces.iter().map(|&f| f as usize).collect(), key));
        }
        let input = MeshInput {
            points,
            counts,
            indices,
            holes,
            left_handed: m.left_handed,
            authored,
            uvs,
            primvars,
            scheme: sd.scheme.clone(),
            authored_level: if tagged { sd.level } else { None },
            boundary: sd.interpolate_boundary.clone(),
            tags,
            base_key,
            subsets,
            motion,
        };
        self.emit_mesh(input, world, inh, out);
        self.t_mesh += t0.elapsed();
    }

    /// `c` is owned: data moved out of the scene layer (see [`take_curve_payloads`]) is
    /// handed on without copying.
    fn curves_scene(&mut self, prim: &rs::Prim, c: rs::Curves, t: f64, world: &M4, inh: &Inherit, out: &mut Emit) {
        if !self.import_curves {
            *self.skipped.entry("BasisCurves (import disabled)".into()).or_default() += 1;
            return;
        }
        self.stats.gprims += 1;
        let t_read = Instant::now();
        let key = self.surface_key_scene(&c.primvars, t, inh);
        let rs::Curves { curve_type, basis, wrap, curve_vertex_counts, points, widths, normals, mut primvars } = c;
        let Some(points) = points.into_at(t) else { return };
        let counts: Vec<i64> = curve_vertex_counts.iter().map(|&n| n as i64).collect();
        if points.is_empty() || counts.is_empty() {
            return;
        }
        // Widths and normals as authored (their rate is inferred from the length)
        let widths: Vec<f32> = widths.and_then(|pv| take_floats(pv, t)).unwrap_or_default();
        let normals: Vec<[f32; 3]> = normals.and_then(|pv| take_float3(pv, t)).unwrap_or_default();
        let st = CURVE_ST_NAMES.iter().find_map(|name| {
            let i = primvars.iter().position(|p| p.name == *name)?;
            let pv = primvars.swap_remove(i);
            let interp = interp_token(pv.interpolation).map(str::to_string);
            take_float2_indexed(pv, t).map(|v| (v, interp))
        });
        let input = CurveInput {
            points,
            counts,
            curve_type: Some(curve_type),
            basis: Some(basis),
            wrap: Some(wrap),
            widths,
            normals,
            st,
            label: prim.path.as_str().to_string(),
            key,
        };
        self.emit_curves(input, world, inh, out, t_read);
    }

    fn points_scene(&mut self, p: &rs::Points, t: f64, world: &M4, inh: &Inherit, out: &mut Emit) {
        self.stats.gprims += 1;
        let Some(pts) = p.points.at(t) else { return };
        let widths: Vec<f32> = p.widths.as_ref().and_then(|pv| raw(pv, t)).map(|(_, v)| v.iter().map(|x| x[0]).collect()).unwrap_or_default();
        let key = self.surface_key_scene(&p.primvars, t, inh);
        self.emit_points(pts, widths, key, world, inh, out);
    }

    fn gprim_scene(&mut self, prim: &rs::Prim, g: &rs::Gprim, t: f64, world: &M4, inh: &Inherit, out: &mut Emit) {
        self.stats.gprims += 1;
        let axis = |a: rs::Axis| match a {
            rs::Axis::X => "X",
            rs::Axis::Y => "Y",
            rs::Axis::Z => "Z",
        };
        let (local, sphere, ax): (Vec<Geom>, bool, &str) = match *g {
            rs::Gprim::Sphere { radius } => (vec![Geom::Sphere { center: P3::origin(), radius: radius as f32 }], true, "Z"),
            rs::Gprim::Cube { size } => (cube(size as f32 * 0.5), false, "Z"),
            rs::Gprim::Cylinder { radius, height, axis: a } => (cylinder(radius as f32, height as f32), false, axis(a)),
            rs::Gprim::Cone { radius, height, axis: a } => (cone(radius as f32, height as f32), false, axis(a)),
            rs::Gprim::Capsule { radius, height, axis: a } => (capsule(radius as f32, height as f32), false, axis(a)),
            rs::Gprim::Plane { width, length, axis: a } => (plane(width as f32, length as f32), false, axis(a)),
        };
        let key = self.surface_key_scene(&prim.primvars, t, inh);
        self.emit_gprim(local, sphere, gprim_axis(ax), key, world, inh, out);
    }

    fn instancer_scene(&mut self, i: &rs::Instancer, t: f64, world: &M4, inh: &Inherit, node: usize, out: &mut Emit) {
        self.stats.point_instancers += 1;
        let input = InstancerInput {
            protos: i.prototypes.iter().map(|p| p.as_str().to_string()).collect(),
            idx: i.proto_indices.iter().map(|&x| x as i64).collect(),
            positions: i.positions.at(t).unwrap_or_default(),
            scales: i.scales.as_ref().and_then(|s| s.at(t)).unwrap_or_default(),
            // Back to (real, i, j, k) for openusd's conversion: identical matrices to the direct path
            orients: i
                .orientations
                .as_ref()
                .and_then(|o| o.at(t))
                .map(|qs| qs.iter().map(|q| crate::xform::quat_wxyz_matrix([q[3] as f64, q[0] as f64, q[1] as f64, q[2] as f64])).collect())
                .unwrap_or_default(),
            invisible: !i.invisible_ids.is_empty(),
        };
        self.emit_point_instancer(input, world, inh, node, out);
    }
}
