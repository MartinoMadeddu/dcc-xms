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
            match load_usd_meshes(Path::new(path)) {
                Ok(meshes) if meshes.is_empty() => None,
                Ok(meshes) => Some(EvalResult::Named(
                    meshes.into_iter()
                        .map(|(path, mesh)| NamedMesh { path, mesh })
                        .collect()
                )),
                Err(e) => {
                    eprintln!("[LoadUsd] failed to load '{}': {}", path, e);
                    None
                }
            }
        }

        NodeType::Transform { translation, rotation, scale } =>
            inputs.first()
                .map(|r| EvalResult::Single(transform(&r.as_mesh(), *translation, *rotation, *scale))),

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

        // ── Modelling ────────────────────────────────────────────────────────
        NodeType::EditPoly { ops, .. } => inputs.first().map(|r| {
            let mesh = crate::core::poly::PolyMesh::from_mesh(&r.as_mesh());
            EvalResult::Single(crate::core::poly::eval_cached(&mesh, ops, ops.len()).to_mesh())
        }),
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
        ..Default::default()
    };
    // Recompute normals after transform so they stay correct
    if !mesh.normals.is_empty() {
        m.compute_normals();
    }
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