use bevy::prelude::{EulerRot, Quat, Vec3};
use crate::types::{EvalResult, MeshData, NamedMesh, NodeType, SubnetId};
use crate::usd_loader::load_usd_meshes;
use std::path::Path;

/// Evaluate one node given its already-resolved upstream inputs.
/// Returns an `EvalResult` — either a single merged mesh or a list of named prims.
pub fn evaluate_node_type(
    node_type:   &NodeType,
    inputs:      &[EvalResult],
    eval_subnet: &impl Fn(SubnetId, &MeshData) -> MeshData,
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

        NodeType::Subnet { id, .. } =>
            inputs.first()
                .map(|r| EvalResult::Single(eval_subnet(*id, &r.as_mesh()))),

        NodeType::Output => inputs.first().cloned(),
    }
}

// ── Generators ────────────────────────────────────────────────────────────────

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
    m
}

pub fn create_grid(rows: u32, cols: u32, size: f32) -> MeshData {
    let mut verts = Vec::new();
    let mut idx   = Vec::new();
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
            idx.extend_from_slice(&[tl, bl, tr, tr, bl, br]);
        }
    }
    let mut m = MeshData::from_triangles(verts, idx);
    m.compute_normals();
    m
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
    let mut m = MeshData {
        vertices:   verts,
        indices:    idx,
        points:     pts,
        ..Default::default()
    };
    m.face_count = m.indices.len() / 3;
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
    for pt in pts {
        let offset = out_verts.len() as u32;
        let t = Vec3::from_array(*pt);
        for v in &template.vertices {
            out_verts.push((Vec3::from_array(*v) + t).to_array());
        }
        out_idx.extend(template.indices.iter().map(|i| i + offset));
    }
    let mut m = MeshData::from_triangles(out_verts, out_idx);
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