use bevy::prelude::{EulerRot, Quat, Vec3};
use crate::types::{Attribute, EvalResult, MeshData, NamedMesh, NodeType, PrimVarInterp, SubnetId};
use crate::usd_loader::load_usd_meshes;
use std::path::Path;

pub fn evaluate_node_type(
    node_type:   &NodeType,
    inputs:      &[EvalResult],
    eval_subnet: &impl Fn(SubnetId, &MeshData, Option<&MeshData>) -> MeshData,
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
                Err(e) => { eprintln!("[LoadUsd] failed to load '{}': {}", path, e); None }
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
            let main_geo     = inputs.first().map(|r| r.as_mesh());
            let template_geo = inputs.get(1).map(|r| r.as_mesh());
            main_geo.map(|geo| {
                EvalResult::Single(eval_subnet(*id, &geo, template_geo.as_ref()))
            })
        }

        NodeType::Output => inputs.first().cloned(),
    }
}

// ── Generators ────────────────────────────────────────────────────────────────
// from_triangles still accepts Vec<[f32;3]> for convenience — no changes needed
// in the generator bodies for vertex data.

pub fn create_cube(size: f32) -> MeshData {
    let s = size / 2.0;
    let mut m = MeshData::from_triangles(
        vec![
            [-s,-s,-s],[s,-s,-s],[s,s,-s],[-s,s,-s],
            [-s,-s, s],[s,-s, s],[s,s, s],[-s,s, s],
        ],
        vec![
            0,1,2,2,3,0,  4,6,5,6,4,7,
            4,5,1,1,0,4,  3,2,6,6,7,3,
            4,0,3,3,7,4,  1,5,6,6,2,1,
        ],
    );
    m.compute_normals();
    m.ensure_standard_attributes();
    m
}

pub fn create_sphere(radius: f32, segments: u32) -> MeshData {
    let mut verts = Vec::new();
    let mut idx   = Vec::new();
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
            idx.extend_from_slice(&[f,s,f+1,s,s+1,f+1]);
        }
    }
    let mut m = MeshData::from_triangles(verts, idx);
    m.compute_normals();
    m.ensure_standard_attributes();
    m
}

pub fn create_grid(rows: u32, cols: u32, size: f32) -> MeshData {
    let mut verts = Vec::new();
    let mut idx   = Vec::new();
    let (rc, cc)  = (rows + 1, cols + 1);
    let (cw, ch)  = (size / cols as f32, size / rows as f32);
    let (ox, oz)  = (-size / 2.0, -size / 2.0);
    for r in 0..rc {
        for c in 0..cc {
            verts.push([ox + c as f32 * cw, 0.0, oz + r as f32 * ch]);
        }
    }
    for r in 0..rows {
        for c in 0..cols {
            let tl = r * cc + c;
            let (tr, bl, br) = (tl + 1, tl + cc, tl + cc + 1);
            idx.extend_from_slice(&[tl, bl, tr, tr, bl, br]);
        }
    }
    let mut m = MeshData::from_triangles(verts, idx);
    m.compute_normals();
    m.ensure_standard_attributes();
    m
}

// ── Operators ─────────────────────────────────────────────────────────────────

pub fn transform(mesh: &MeshData, t: Vec3, r: Vec3, s: Vec3) -> MeshData {
    let rot = Quat::from_euler(EulerRot::XYZ, r.x, r.y, r.z);

    // Positions: Vec3 directly now — no from_array / to_array needed
    let new_positions: Vec<Vec3> = mesh.positions.values.iter()
        .map(|v| rot * (*v * s) + t)
        .collect();

    // Points: also Vec3
    let new_points: Vec<Vec3> = mesh.points.iter()
        .map(|p| rot * (*p * s) + t)
        .collect();

    let mut m = MeshData {
        positions: Attribute::vertex("P", new_positions),
        indices:   mesh.indices.clone(),
        points:    new_points,
        face_count: mesh.face_count,
        ..Default::default()
    };

    // Recompute normals if the source had them
    if mesh.normals.is_some() {
        m.compute_normals();
    }
    m
}

pub fn merge(a: &MeshData, b: &MeshData) -> MeshData {
    let mut positions = a.positions.values.clone();
    let off           = positions.len() as u32;
    positions.extend_from_slice(&b.positions.values);

    let mut indices = a.indices.clone();
    indices.extend(b.indices.iter().map(|i| i + off));

    let mut points = a.points.clone();
    points.extend_from_slice(&b.points);

    let face_count = indices.len() / 3;

    let mut m = MeshData {
        positions:  Attribute::vertex("P", positions),
        indices,
        points,
        face_count,
        ..Default::default()
    };
    m.compute_normals();
    m
}

pub fn scatter_points(mesh: &MeshData, count: u32, seed: u32) -> MeshData {
    let mut pts = Vec::with_capacity(count as usize);
    let mut rng = LcgRng::new(seed);
    let n       = mesh.positions.len();

    // Collect triangles as (Vec3, Vec3, Vec3) — no array conversion needed
    let tris: Vec<(Vec3, Vec3, Vec3)> = mesh.indices
        .chunks(3)
        .filter_map(|c| {
            if c.len() < 3 { return None; }
            let (ai, bi, ci) = (c[0] as usize, c[1] as usize, c[2] as usize);
            if ai < n && bi < n && ci < n {
                Some((
                    mesh.positions.values[ai],
                    mesh.positions.values[bi],
                    mesh.positions.values[ci],
                ))
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
        pts.push(a * r3 + b * r1 + c * r2);
    }

    MeshData {
        positions: Attribute::vertex("P", vec![]),
        points:    pts,
        ..Default::default()
    }
}

pub fn copy_to_points(template: &MeshData, point_cloud: &MeshData) -> MeshData {
    // Use scatter points if present, otherwise fall back to mesh positions
    let pts: &[Vec3] = if !point_cloud.points.is_empty() {
        &point_cloud.points
    } else {
        &point_cloud.positions.values
    };

    let template_verts = &template.positions.values;
    let mut out_pos  = Vec::with_capacity(pts.len() * template_verts.len());
    let mut out_idx  = Vec::with_capacity(pts.len() * template.indices.len());

    for pt in pts {
        let offset = out_pos.len() as u32;
        out_pos.extend(template_verts.iter().map(|v| *v + *pt));
        out_idx.extend(template.indices.iter().map(|i| i + offset));
    }

    let face_count = out_idx.len() / 3;
    let mut m = MeshData {
        positions:  Attribute::vertex("P", out_pos),
        indices:    out_idx,
        face_count,
        ..Default::default()
    };
    m.compute_normals();
    m.ensure_standard_attributes();
    m
}

// ── LCG RNG (unchanged) ───────────────────────────────────────────────────────

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