use bevy::prelude::{EulerRot, Quat, Vec3};
use crate::types::{EvalResult, MeshData, NamedMesh, NodeType, RetimeMode, SubnetId};
use crate::core::anim::{self, AnimData, FrameRate};
use std::sync::Arc;
use crate::usd_loader::load_usd_meshes;
use std::path::Path;

/// Evaluate one node given its already-resolved upstream inputs.
/// Returns an `EvalResult` — either a single merged mesh or a list of named prims.
/// `output` is the index of the output socket being asked for. Only nodes
/// with several outputs look at it.
pub fn evaluate_node_type(
    node_type:   &NodeType,
    inputs:      &[EvalResult],
    eval_subnet: &impl Fn(SubnetId, &MeshData, Option<&MeshData>) -> MeshData,
    output:      usize,
) -> Option<EvalResult> {
    match node_type {
        NodeType::CreateCube   { size }             => Some(EvalResult::Single(create_cube(*size))),
        NodeType::CreateSphere { radius, segments } => Some(EvalResult::Single(create_sphere(*radius, *segments))),
        NodeType::CreateGrid   { rows, cols, size } => Some(EvalResult::Single(create_grid(*rows, *cols, *size))),

        NodeType::LoadUsd { path } => {
            if path.is_empty() { return None; }
            // Packed primitives: one per mesh prim, shared with the cached stage.
            match crate::usd_scene::load_cached(path) {
                Ok(scene) if scene.meshes.is_empty() => None,
                Ok(scene) => {
                    let looks = scene.looks();
                    Some(EvalResult::Named(scene.meshes.iter().map(|m| NamedMesh {
                        path: m.path.clone(), mesh: m.mesh.clone(), picked: false, material: m.material.clone(),
                        look: m.material.as_ref().and_then(|p| looks.get(p).cloned()),
                    }).collect()))
                }
                Err(_) => None,
            }
        }

        NodeType::PickPrims { pattern } => inputs.first().map(|r| match r {
            EvalResult::Named(prims) => {
                let p = crate::core::pattern::NamePattern::new(pattern);
                EvalResult::Named(prims.iter().map(|m| NamedMesh { picked: p.matches(&m.path), ..m.clone() }).collect())
            }
            other => other.clone(),
        }),
        NodeType::PrunePrims { pattern, keep } => inputs.first().map(|r| match r {
            EvalResult::Named(prims) => {
                let p = crate::core::pattern::NamePattern::new(pattern);
                if p.is_empty() { return r.clone(); }
                EvalResult::Named(prims.iter().filter(|m| p.matches(&m.path) == *keep).cloned().collect())
            }
            other => other.clone(),
        }),
        NodeType::UnpackPrims => inputs.first().map(|r| EvalResult::Single(r.as_mesh())),

        NodeType::Transform { translation, rotation, scale } => inputs.first().and_then(|r| match r {
            // A clip moves by its top joints; its skin follows when posed.
            EvalResult::Anim(clip) => {
                let x = transform_matrix(*translation, *rotation, Vec3::splat(clip_scale(*scale)));
                anim::memo(&format!("{node_type:?}"), Some(clip), || Some(clip.moved(x))).map(EvalResult::Anim)
            }
            // Packed primitives with none picked: each one moves and stays
            // a primitive of its own.
            EvalResult::Named(prims) if !prims.iter().any(|p| p.picked) => Some(EvalResult::Named(prims.iter().map(|p| NamedMesh {
                mesh: Arc::new(transform(&p.mesh, *translation, *rotation, *scale)), ..p.clone()
            }).collect())),
            other => Some(other.map_mesh(|m| transform(&m, *translation, *rotation, *scale))),
        }),

        NodeType::Merge => {
            let meshes: Vec<MeshData> = inputs.iter().map(|r| r.as_mesh()).collect();
            if meshes.len() >= 2 {
                Some(EvalResult::Single(merge(&meshes[0], &meshes[1])))
            } else {
                meshes.into_iter().next().map(EvalResult::Single)
            }
        }

        NodeType::ScatterPoints { count, seed } =>
            inputs.first()
                .map(|r| EvalResult::Single(scatter_points(&r.as_mesh(), *count, *seed))),

        NodeType::CopyToPoints =>
            if inputs.len() >= 2 {
                Some(EvalResult::Single(copy_to_points(&inputs[0].as_mesh(), &inputs[1].as_mesh())))
            } else { None },

        NodeType::Subnet { id, .. } => {
            // First input is the main geometry, second (if exists) is template
            let main_geo = inputs.first().map(|r| r.as_mesh());
            let template_geo = inputs.get(1).map(|r| r.as_mesh());
            
            main_geo.map(|geo| {
                EvalResult::Single(eval_subnet(*id, &geo, template_geo.as_ref()))
            })
        }

        NodeType::Output => inputs.first().cloned(),

        // ── Animation ────────────────────────────────────────────────────────
        NodeType::LoadFbx { path, take } =>
            crate::fbx_loader::load_fbx_cached(path, *take).ok()
                .map(|l| EvalResult::Anim(l.anim)),

        NodeType::TestClip { seconds, fps_num, fps_den } =>
            anim::memo(&format!("{node_type:?}"), None, || {
                Some(anim::create_test_clip(*seconds, FrameRate::new(*fps_num, *fps_den)))
            }).map(EvalResult::Anim),

        NodeType::RenameJoints { find, replace, strip_namespace, prefix } =>
            anim_op(node_type, inputs, |a| a.renamed(find, replace, *strip_namespace, prefix)),

        NodeType::TrimClip { head, tail } =>
            anim_op(node_type, inputs, |a| a.trimmed(*head as usize, *tail as usize)),

        NodeType::Retime { fps_num, fps_den, mode } =>
            anim_op(node_type, inputs, |a| {
                let rate = FrameRate::new(*fps_num, *fps_den);
                match mode {
                    RetimeMode::Resample    => a.resampled(rate),
                    RetimeMode::Reinterpret => a.reinterpreted(rate),
                }
            }),

        NodeType::SetTimecode { hours, minutes, seconds, frames, drop_frame } =>
            anim_op(node_type, inputs, |a| {
                a.with_start_timecode(*hours, *minutes, *seconds, *frames, *drop_frame)
            }),

        // ── Batch / export ───────────────────────────────────────────────────
        NodeType::LoadFbxDir { dir, index, take } => {
            let files = crate::fbx_loader::list_fbx(dir);
            let path  = files.get((*index as usize).min(files.len().checked_sub(1)?))?;
            crate::fbx_loader::load_fbx_cached(&path.to_string_lossy(), *take).ok()
                .map(|l| EvalResult::Anim(l.anim))
        }

        NodeType::SplitSkeleton { picks } => {
            let input = inputs.first()?.as_anim()?;
            let root  = split_root(input, picks.get(output)?)?;
            anim::memo(&format!("split:{root}"), Some(input), || Some(input.split(root)))
                .map(EvalResult::Anim)
        }

        NodeType::AutoTPose { set_hip_height, hip_height } =>
            anim_op(node_type, inputs, |a| {
                a.auto_tpose(set_hip_height.then_some(*hip_height * 0.01))
            }),

        NodeType::FixPose { edits } =>
            anim_op(node_type, inputs, |a| a.pose_fixed(edits)),

        NodeType::ProxySkin { thickness } =>
            anim_op(node_type, inputs, |a| a.with_proxy_skin(*thickness)),

        NodeType::WriteFbx { .. } => inputs.first().cloned(),

        // ── Mocap tools ──────────────────────────────────────────────────────
        NodeType::MirrorClip => anim_op(node_type, inputs, |a| a.mirrored()),
        NodeType::SmoothClip { radius, amount, translations } =>
            anim_op(node_type, inputs, |a| a.smoothed(*radius, *amount, *translations)),
        NodeType::InPlace { keep_height, to_root } =>
            anim_op(node_type, inputs, |a| a.in_place(*keep_height, *to_root)),
        // Read from older graphs only: `graph_io` turns it into Transform.
        NodeType::TransformClip { translate, rotate, scale } =>
            anim_op(node_type, inputs, |a| a.transformed(Vec3::from_array(*translate), Vec3::from_array(*rotate), *scale)),
        NodeType::LoopClip { blend } => anim_op(node_type, inputs, |a| a.looped(*blend)),
        NodeType::TimeWarp { speed, reverse } => anim_op(node_type, inputs, |a| a.time_warped(*speed, *reverse)),
        NodeType::PruneJoints { words } => anim_op(node_type, inputs, |a| a.pruned(words)),
        NodeType::FloorClip { height } => anim_op(node_type, inputs, |a| a.floored(*height)),
        NodeType::BlendClips { blend, align } =>
            anim_op2(node_type, inputs, |a, b| a.blended(b, *blend, *align)),
        NodeType::Retarget => anim_op2(node_type, inputs, |a, b| a.retargeted(b).0),
        NodeType::Characterize { picks } => anim_op(node_type, inputs, |a| crate::core::human::with_picks(a, picks)),

        // ── Ragdoll ──────────────────────────────────────────────────────────
        NodeType::LoadFbxMesh { path } => {
            let meshes = crate::fbx_loader::load_meshes_cached(path).ok()?;
            Some(EvalResult::Named(meshes.iter().map(|(name, mesh)| NamedMesh {
                path: format!("/{name}"), mesh: mesh.clone(), picked: false, material: None, look: None,
            }).collect()))
        }
        NodeType::Calamari { hulls, detail } =>
            anim_op(node_type, inputs, |a| crate::ragdoll::calamari(a, *hulls, *detail)),
        NodeType::Ragdoll { settings, limits, .. } => {
            // The clip, and the collider when one is wired. Nothing is solved
            // here: the node puts out the result of a solve when there is
            // one for exactly these inputs and settings.
            let clip = inputs.iter().find_map(|r| r.as_anim())?;
            let collider = inputs.iter().find(|r| r.as_anim().is_none()).map(|r| r.shared_mesh());
            Some(EvalResult::Anim(crate::ragdoll::output(clip, collider.as_ref(), settings, limits)))
        }

        // ── UV ───────────────────────────────────────────────────────────────
        NodeType::UvUnwrap { method, angle, margin, axis, tiles } => inputs.first().map(|r| r.map_mesh(|mut mesh| {
            mesh.uvs = (*crate::core::uv::unwrap_cached(&mesh, *method, *angle, *margin, *axis, *tiles)).clone();
            mesh
        })),
        NodeType::UvTransform { offset, rotate, scale } => inputs.first().map(|r| r.map_mesh(|mut mesh| {
            mesh.uvs = crate::core::uv::transform_all(&mesh, *offset, *rotate, *scale);
            mesh
        })),
        NodeType::UvEdit { edits } => inputs.first().map(|r| r.map_mesh(|mut mesh| {
            mesh.uvs = crate::core::uv::edit_islands(&mesh, edits);
            mesh
        })),

        // ── Modelling ────────────────────────────────────────────────────────
        // On packed primitives with some picked, only those are edited.
        NodeType::EditPoly { ops, .. } => inputs.first().map(|r| r.map_mesh(|m| {
            let mesh = crate::core::poly::PolyMesh::from_mesh(&m);
            crate::core::poly::eval_cached(&mesh, ops, ops.len()).to_mesh()
        })),
    }
}

/// Joint index a Split output refers to in this clip.
pub fn split_root(clip: &AnimData, pick: &crate::types::SplitPick) -> Option<usize> {
    match pick {
        crate::types::SplitPick::Character(i) => clip.character_roots().get(*i as usize).copied(),
        crate::types::SplitPick::Joint(name)  => clip.joints.iter().position(|j| j.name == *name),
    }
}

/// Run a clip operator on the first input. Returns nothing when the input is
/// not animation. The result is memoised on the node parameters and the input.
fn anim_op(
    node_type: &NodeType,
    inputs:    &[EvalResult],
    op:        impl FnOnce(&AnimData) -> AnimData,
) -> Option<EvalResult> {
    let input: &Arc<AnimData> = inputs.first()?.as_anim()?;
    anim::memo(&format!("{node_type:?}"), Some(input), || Some(op(input)))
        .map(EvalResult::Anim)
}

/// A clip operator with two clip inputs. Needs both wired.
fn anim_op2(
    node_type: &NodeType,
    inputs:    &[EvalResult],
    op:        impl FnOnce(&AnimData, &AnimData) -> AnimData,
) -> Option<EvalResult> {
    let a: &Arc<AnimData> = inputs.first()?.as_anim()?;
    let b: &Arc<AnimData> = inputs.get(1)?.as_anim()?;
    // The second input is part of the key by its address.
    anim::memo(&format!("{node_type:?}:{:p}", Arc::as_ptr(b)), Some(a), || Some(op(a, b)))
        .map(EvalResult::Anim)
}

// ── Generators ────────────────────────────────────────────────────────────────

pub fn create_cube(size: f32) -> MeshData {
    let s = size / 2.0;
    // Six quads, counter-clockwise seen from outside.
    MeshData::from_polys(
        vec![
            [-s,-s,-s],[s,-s,-s],[s,s,-s],[-s,s,-s],
            [-s,-s, s],[s,-s, s],[s,s, s],[-s,s, s],
        ],
        vec![
            vec![0,3,2,1], vec![4,5,6,7],   // -Z, +Z
            vec![0,1,5,4], vec![3,7,6,2],   // -Y, +Y
            vec![0,4,7,3], vec![1,2,6,5],   // -X, +X
        ],
    )
}

pub fn create_sphere(radius: f32, segments: u32) -> MeshData {
    let mut verts = Vec::new();
    let mut idx   = Vec::new();
    let mut quads = Vec::new();
    for lat in 0..=segments {
        let theta    = lat as f32 * std::f32::consts::PI / segments as f32;
        let (st, ct) = (theta.sin(), theta.cos());
        for lon in 0..=segments {
            let phi = lon as f32 * 2.0 * std::f32::consts::PI / segments as f32;
            verts.push([phi.cos()*st*radius, ct*radius, phi.sin()*st*radius]);
        }
    }
    for lat in 0..segments {
        for lon in 0..segments {
            let f = lat*(segments+1)+lon;
            let s = f+segments+1;
            // Counter-clockwise seen from outside, like the cube and the grid.
            idx.extend_from_slice(&[f,f+1,s,s,f+1,s+1]);
            quads.push(vec![f, f+1, s+1, s]);
        }
    }
    let mut m = MeshData::from_triangles(verts, idx);
    m.face_count = quads.len();
    m.polys = quads;
    // On a sphere the normal is the direction from the centre. This also
    // covers the poles, where the triangles have no area.
    m.normals = m.vertices.iter().map(|v| Vec3::from_array(*v).normalize_or(Vec3::Y).to_array()).collect();
    m
}

pub fn create_grid(rows: u32, cols: u32, size: f32) -> MeshData {
    let mut verts = Vec::new();
    let mut quads = Vec::new();
    let rc = rows + 1;
    let cc = cols + 1;
    let cw = size / cols as f32;
    let ch = size / rows as f32;
    let ox = -size / 2.0;
    let oz = -size / 2.0;
    for r in 0..rc {
        for c in 0..cc {
            verts.push([ox + c as f32 * cw, 0.0, oz + r as f32 * ch]);
        }
    }
    for r in 0..rows {
        for c in 0..cols {
            let tl = r * cc + c;
            let tr = tl + 1;
            let bl = tl + cc;
            let br = bl + 1;
            quads.push(vec![tl, bl, br, tr]);
        }
    }
    MeshData::from_polys(verts, quads)
}

// ── Operators ─────────────────────────────────────────────────────────────────

/// The matrix of a Transform node: scale, then rotation X, Y, Z (radians),
/// then translation.
pub fn transform_matrix(t: Vec3, r: Vec3, s: Vec3) -> bevy::math::Mat4 {
    bevy::math::Mat4::from_scale_rotation_translation(s, Quat::from_euler(EulerRot::XYZ, r.x, r.y, r.z), t)
}

/// A skeleton scales the same on every axis: a clip takes the X scale.
pub fn clip_scale(s: Vec3) -> f32 { if s.x.abs() > 1e-6 { s.x } else { 1e-6 } }

pub fn transform(mesh: &MeshData, t: Vec3, r: Vec3, s: Vec3) -> MeshData {
    let rot = Quat::from_euler(EulerRot::XYZ, r.x, r.y, r.z);
    let mut m = MeshData {
        vertices: mesh.vertices.iter()
            .map(|v| (rot * (Vec3::from_array(*v) * s) + t).to_array())
            .collect(),
        indices:  mesh.indices.clone(),
        points:   mesh.points.iter()
            .map(|p| (rot * (Vec3::from_array(*p) * s) + t).to_array())
            .collect(),
        polys:      mesh.polys.clone(),
        face_count: mesh.face_count,
        uvs:        mesh.uvs.clone(),
        ..Default::default()
    };
    // Recompute normals after transform so they stay correct
    if !mesh.normals.is_empty() {
        m.compute_normals();
    }
    m
}

/// Any number of meshes as one, in a single pass.
pub fn merge_all(parts: &[&MeshData]) -> MeshData {
    match parts {
        [] => return MeshData::default(),
        [one] => return (*one).clone(),
        _ => {}
    }
    let mut m = MeshData::default();
    m.vertices.reserve(parts.iter().map(|p| p.vertices.len()).sum());
    m.indices.reserve(parts.iter().map(|p| p.indices.len()).sum());
    let any_polys = parts.iter().any(|p| !p.polys.is_empty());
    let any_uvs = parts.iter().any(|p| !p.uvs.is_empty());
    let all_normals = parts.iter().all(|p| p.normals.len() == p.vertices.len());
    for p in parts {
        let off = m.vertices.len() as u32;
        m.vertices.extend_from_slice(&p.vertices);
        m.indices.extend(p.indices.iter().map(|i| i + off));
        m.points.extend_from_slice(&p.points);
        if any_polys {
            if p.polys.is_empty() { m.polys.extend(p.indices.chunks_exact(3).map(|t| t.iter().map(|i| i + off).collect::<Vec<u32>>())); }
            else { m.polys.extend(p.polys.iter().map(|poly| poly.iter().map(|i| i + off).collect::<Vec<u32>>())); }
        }
        if any_uvs {
            if p.uvs.len() == p.indices.len() { m.uvs.extend_from_slice(&p.uvs); }
            else { m.uvs.extend(std::iter::repeat([0.0; 2]).take(p.indices.len())); }
        }
        if all_normals { m.normals.extend_from_slice(&p.normals); }
    }
    m.face_count = if m.polys.is_empty() { m.indices.len() / 3 } else { m.polys.len() };
    if !all_normals { m.compute_normals(); }
    m
}

pub fn merge(a: &MeshData, b: &MeshData) -> MeshData {
    let mut verts = a.vertices.clone();
    let mut idx   = a.indices.clone();
    let off = verts.len() as u32;
    verts.extend(&b.vertices);
    idx.extend(b.indices.iter().map(|i| i + off));
    let mut pts = a.points.clone();
    pts.extend(&b.points);
    // Polygons survive when either side has them.
    let mut polys = vec![];
    if !a.polys.is_empty() || !b.polys.is_empty() {
        polys = a.polygons();
        polys.extend(b.polygons().into_iter().map(|p| p.into_iter().map(|i| i + off).collect::<Vec<u32>>()));
    }
    let mut m = MeshData {
        vertices:   verts,
        indices:    idx,
        points:     pts,
        ..Default::default()
    };
    m.face_count = if polys.is_empty() { m.indices.len() / 3 } else { polys.len() };
    m.polys = polys;
    // UVs survive when either side has them; the other side gets zeros.
    if !a.uvs.is_empty() || !b.uvs.is_empty() {
        let side = |x: &MeshData| if x.uvs.len() == x.indices.len() { x.uvs.clone() } else { vec![[0.0; 2]; x.indices.len()] };
        m.uvs = side(a);
        m.uvs.extend(side(b));
    }
    m.compute_normals();
    m
}

pub fn scatter_points(mesh: &MeshData, count: u32, seed: u32) -> MeshData {
    let mut pts = Vec::with_capacity(count as usize);
    let mut rng = LcgRng::new(seed);
    let tris: Vec<([f32; 3], [f32; 3], [f32; 3])> = mesh.indices
        .chunks(3)
        .filter_map(|c| {
            if c.len() < 3 { return None; }
            let (a, b, d) = (c[0] as usize, c[1] as usize, c[2] as usize);
            if a < mesh.vertices.len() && b < mesh.vertices.len() && d < mesh.vertices.len() {
                Some((mesh.vertices[a], mesh.vertices[b], mesh.vertices[d]))
            } else { None }
        })
        .collect();
    if tris.is_empty() { return MeshData::default(); }
    for _ in 0..count {
        let ti = rng.next_u32() as usize % tris.len();
        let (a, b, c) = tris[ti];
        let mut r1 = rng.next_f32();
        let mut r2 = rng.next_f32();
        if r1 + r2 > 1.0 { r1 = 1.0 - r1; r2 = 1.0 - r2; }
        let r3 = 1.0 - r1 - r2;
        pts.push([
            a[0]*r3 + b[0]*r1 + c[0]*r2,
            a[1]*r3 + b[1]*r1 + c[1]*r2,
            a[2]*r3 + b[2]*r1 + c[2]*r2,
        ]);
    }
    MeshData {
        vertices: vec![],
        indices:  vec![],
        points:   pts,
        ..Default::default()
    }
}

pub fn copy_to_points(template: &MeshData, point_cloud: &MeshData) -> MeshData {
    let pts = if !point_cloud.points.is_empty() {
        &point_cloud.points
    } else {
        &point_cloud.vertices
    };
    let mut out_verts = Vec::new();
    let mut out_idx   = Vec::new();
    let mut out_polys = Vec::new();
    for pt in pts {
        let offset = out_verts.len() as u32;
        let t = Vec3::from_array(*pt);
        for v in &template.vertices {
            out_verts.push((Vec3::from_array(*v) + t).to_array());
        }
        out_idx.extend(template.indices.iter().map(|i| i + offset));
        out_polys.extend(template.polys.iter().map(|p| p.iter().map(|i| i + offset).collect::<Vec<u32>>()));
    }
    let mut m = MeshData::from_triangles(out_verts, out_idx);
    if !out_polys.is_empty() {
        m.face_count = out_polys.len();
        m.polys = out_polys;
    }
    m.compute_normals();
    m
}

struct LcgRng(u64);
impl LcgRng {
    fn new(seed: u32) -> Self { Self(seed as u64 | 1) }
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005)
                       .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn next_f32(&mut self) -> f32 { self.next_u32() as f32 / u32::MAX as f32 }
}
#[cfg(test)]
mod packed_tests {
    use super::*;
    use crate::types::{NodeType, SubnetId};

    fn pass(_: SubnetId, m: &MeshData, _: Option<&MeshData>) -> MeshData { m.clone() }
    fn run(node: NodeType, input: &EvalResult) -> EvalResult {
        evaluate_node_type(&node, std::slice::from_ref(input), &pass, 0).unwrap()
    }
    /// Three unit cubes side by side, as packed primitives.
    fn packed() -> EvalResult {
        EvalResult::Named((0..3).map(|i| {
            let mesh = transform(&create_cube(1.0), bevy::math::Vec3::new(i as f32 * 5.0, 0.0, 0.0), bevy::math::Vec3::ZERO, bevy::math::Vec3::ONE);
            NamedMesh::new(format!("/car/{}", ["body", "wheel_L", "wheel_R"][i]), mesh)
        }).collect())
    }
    fn prims(r: &EvalResult) -> &[NamedMesh] { match r { EvalResult::Named(p) => p, _ => panic!("not packed") } }
    fn max_y(m: &MeshData) -> f32 { m.vertices.iter().map(|v| v[1]).fold(f32::MIN, f32::max) }

    #[test]
    fn pick_marks_by_path_pattern_and_shares_the_meshes() {
        let input = packed();
        let out = run(NodeType::PickPrims { pattern: "wheel_L$".into() }, &input);
        let picked: Vec<bool> = prims(&out).iter().map(|p| p.picked).collect();
        assert_eq!(picked, vec![false, true, false]);
        assert!(out.has_picked() && !input.has_picked());
        // Nothing is copied on the way through.
        assert!(prims(&out).iter().zip(prims(&input)).all(|(a, b)| std::sync::Arc::ptr_eq(&a.mesh, &b.mesh)));
        assert_eq!(out.work_mesh().vertices.len(), create_cube(1.0).vertices.len());
    }

    #[test]
    fn a_mesh_node_after_a_pick_changes_only_the_picked() {
        let input = packed();
        let picked = run(NodeType::PickPrims { pattern: "wheel".into() }, &input);
        let lift = NodeType::Transform { translation: bevy::math::Vec3::new(0.0, 10.0, 0.0), rotation: bevy::math::Vec3::ZERO, scale: bevy::math::Vec3::ONE };
        let out = run(lift.clone(), &picked);
        // Two wheels come out as one picked primitive, in the first one's place.
        let p = prims(&out);
        assert_eq!(p.iter().map(|x| x.path.as_str()).collect::<Vec<_>>(), vec!["/car/body", "/car/wheel_L"]);
        assert!(std::sync::Arc::ptr_eq(&p[0].mesh, &prims(&input)[0].mesh), "the body is passed through as it is");
        assert!(max_y(&p[0].mesh) < 1.0 && max_y(&p[1].mesh) > 10.0 && p[1].picked);
        assert_eq!(p[1].mesh.vertices.len(), 2 * create_cube(1.0).vertices.len());
        // Without a pick Transform moves every primitive and keeps them apart.
        let all = run(lift, &input);
        assert_eq!(prims(&all).len(), prims(&input).len());
        assert!(prims(&all).iter().all(|p| p.mesh.vertices.iter().all(|v| v[1] > 9.0)));
    }

    #[test]
    fn edit_poly_after_a_pick_edits_the_picked_primitive() {
        let picked = run(NodeType::PickPrims { pattern: "body".into() }, &packed());
        let out = run(NodeType::EditPoly { ops: vec![], pending: Default::default(), edit: None, auto_collapse: false }, &picked);
        assert_eq!(prims(&out).len(), 3);
        assert!(prims(&out)[0].picked && prims(&out)[0].mesh.vertices.len() >= 8);
    }

    #[test]
    fn prune_removes_or_keeps_and_unpack_merges() {
        let input = packed();
        let without = run(NodeType::PrunePrims { pattern: "wheel".into(), keep: false }, &input);
        assert_eq!(prims(&without).len(), 1);
        let only = run(NodeType::PrunePrims { pattern: "wheel".into(), keep: true }, &input);
        assert_eq!(prims(&only).len(), 2);
        // An empty pattern leaves everything.
        assert_eq!(prims(&run(NodeType::PrunePrims { pattern: " ".into(), keep: true }, &input)).len(), 3);
        let one = run(NodeType::UnpackPrims, &input);
        assert!(matches!(&one, EvalResult::Single(m) if m.vertices.len() == 3 * create_cube(1.0).vertices.len()));
    }

    #[test]
    fn merging_many_at_once_matches_merging_in_pairs() {
        let parts: Vec<MeshData> = prims(&packed()).iter().map(|p| (*p.mesh).clone()).collect();
        let refs: Vec<&MeshData> = parts.iter().collect();
        let all = merge_all(&refs);
        let pairs = merge(&merge(&parts[0], &parts[1]), &parts[2]);
        assert_eq!((all.vertices.len(), all.indices.len(), all.polygons().len()), (pairs.vertices.len(), pairs.indices.len(), pairs.polygons().len()));
        assert_eq!(all.indices, pairs.indices);
        assert_eq!(all.normals.len(), all.vertices.len());
    }
}
