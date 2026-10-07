//! UV coordinates: unwrapping, island transforms and what the UV editor
//! needs to draw them.
//!
//! UVs live on `MeshData::uvs`, one pair per triangle corner, in the order
//! of `MeshData::indices`. Two triangles share a UV edge when they agree on
//! the UVs at both of its ends; where they do not, there is a seam.
//!
//! The unwrap is Least Squares Conformal Maps (Lévy, Petitjean, Ray and
//! Maillot, "Least Squares Conformal Maps for Automatic Texture Atlas
//! Generation", SIGGRAPH 2002): each chart is flattened so that angles are
//! kept as well as a flat sheet allows, by solving one linear least-squares
//! problem with two pinned vertices. Charts come from growing regions whose
//! normals stay within an angle of the region's first triangle, and are
//! packed into the unit square at equal texel density.

use std::collections::HashMap;

use bevy::math::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::types::MeshData;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UvMethod {
    /// Charts by normal angle, each flattened with LSCM, packed.
    Conformal,
    /// Six projections along the axes, packed.
    Box,
    /// One projection along an axis.
    Planar,
}

/// One edit of the UV Edit node: a transform of one island about its centre.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IslandEdit {
    pub island: u32,
    pub offset: [f32; 2],
    /// Degrees, counter-clockwise.
    pub rotate: f32,
    pub scale:  [f32; 2],
}

impl IslandEdit {
    pub fn new(island: u32) -> Self { Self { island, offset: [0.0; 2], rotate: 0.0, scale: [1.0; 2] } }
}

fn tri_count(mesh: &MeshData) -> usize { mesh.indices.len() / 3 }
fn pos(mesh: &MeshData, v: u32) -> Vec3 { Vec3::from_array(mesh.vertices[v as usize]) }
fn tri(mesh: &MeshData, t: usize) -> [u32; 3] { [mesh.indices[t * 3], mesh.indices[t * 3 + 1], mesh.indices[t * 3 + 2]] }
fn tri_normal_area(mesh: &MeshData, t: usize) -> (Vec3, f32) {
    let [a, b, c] = tri(mesh, t);
    let n = (pos(mesh, b) - pos(mesh, a)).cross(pos(mesh, c) - pos(mesh, a));
    (n.normalize_or_zero(), n.length() * 0.5)
}
fn edge_key(a: u32, b: u32) -> (u32, u32) { if a < b { (a, b) } else { (b, a) } }

// ============================================================================
// CHARTS
// ============================================================================

/// Split the triangles into charts: connected regions whose normals stay
/// within `angle_deg` of the normal of the region's first triangle. Returns
/// the chart of each triangle and the number of charts.
pub fn charts_by_angle(mesh: &MeshData, angle_deg: f32) -> (Vec<usize>, usize) {
    let nt = tri_count(mesh);
    let info: Vec<(Vec3, f32)> = (0..nt).map(|t| tri_normal_area(mesh, t)).collect();
    let mut by_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for t in 0..nt {
        let v = tri(mesh, t);
        for k in 0..3 { by_edge.entry(edge_key(v[k], v[(k + 1) % 3])).or_default().push(t); }
    }
    let limit = angle_deg.clamp(1.0, 89.0).to_radians().cos();
    // Largest triangles seed first, so charts start on the broad faces.
    let mut order: Vec<usize> = (0..nt).collect();
    order.sort_by(|a, b| info[*b].1.total_cmp(&info[*a].1).then(a.cmp(b)));
    let mut chart = vec![usize::MAX; nt];
    let mut count = 0;
    for seed in order {
        if chart[seed] != usize::MAX { continue; }
        let normal = info[seed].0;
        chart[seed] = count;
        let mut todo = vec![seed];
        while let Some(t) = todo.pop() {
            let v = tri(mesh, t);
            for k in 0..3 {
                for n in &by_edge[&edge_key(v[k], v[(k + 1) % 3])] {
                    // A triangle with no area has no normal: it goes with its neighbour.
                    if chart[*n] == usize::MAX && (info[*n].1 < 1e-10 || info[*n].0.dot(normal) >= limit) {
                        chart[*n] = count;
                        todo.push(*n);
                    }
                }
            }
        }
        count += 1;
    }
    (chart, count)
}

// ============================================================================
// LSCM
// ============================================================================

/// Flatten one chart. `tris` are triangles as local vertex indices into
/// `points`. Returns one UV per point.
pub fn lscm(points: &[Vec3], tris: &[[usize; 3]]) -> Vec<Vec2> {
    let n = points.len();
    if n < 3 || tris.is_empty() { return vec![Vec2::ZERO; n]; }

    // Pin the two points furthest apart along the chart's longest axis.
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in points { lo = lo.min(*p); hi = hi.max(*p); }
    let ext = hi - lo;
    let axis = if ext.x >= ext.y && ext.x >= ext.z { 0 } else if ext.y >= ext.z { 1 } else { 2 };
    let pin_a = (0..n).min_by(|a, b| points[*a][axis].total_cmp(&points[*b][axis])).unwrap();
    let pin_b = (0..n).max_by(|a, b| points[*a][axis].total_cmp(&points[*b][axis])).unwrap();
    if pin_a == pin_b { return vec![Vec2::ZERO; n]; }
    let span = points[pin_a].distance(points[pin_b]).max(1e-9);
    let pinned = |v: usize| -> Option<Vec2> {
        if v == pin_a { Some(Vec2::ZERO) } else if v == pin_b { Some(Vec2::new(span, 0.0)) } else { None }
    };

    // Unknowns: u and v of every free point.
    let mut column = vec![usize::MAX; n];
    let mut free = 0;
    for v in 0..n { if pinned(v).is_none() { column[v] = free; free += 1; } }
    if free == 0 { return (0..n).map(|v| pinned(v).unwrap()).collect(); }

    // Two rows per triangle: the real and imaginary part of
    // sum_j W_j (u_j + i v_j) = 0, with W_j the opposite edge of corner j
    // in the triangle's own plane, scaled by 1 / sqrt(2 area).
    let mut rows: Vec<Vec<(usize, f64)>> = Vec::with_capacity(tris.len() * 2);
    let mut rhs:  Vec<f64> = Vec::with_capacity(tris.len() * 2);
    for t in tris {
        let (p0, p1, p2) = (points[t[0]], points[t[1]], points[t[2]]);
        let x_axis = (p1 - p0).normalize_or_zero();
        let normal = x_axis.cross(p2 - p0);
        let double_area = normal.length();
        if double_area < 1e-12 { continue; }
        let y_axis = normal.normalize().cross(x_axis);
        let flat = [Vec2::ZERO, Vec2::new((p1 - p0).dot(x_axis), 0.0), Vec2::new((p2 - p0).dot(x_axis), (p2 - p0).dot(y_axis))];
        let s = 1.0 / (double_area as f64).sqrt();
        let w = [flat[2] - flat[1], flat[0] - flat[2], flat[1] - flat[0]];
        let (mut real, mut imag): (Vec<(usize, f64)>, Vec<(usize, f64)>) = (vec![], vec![]);
        let (mut b_real, mut b_imag) = (0.0f64, 0.0f64);
        for j in 0..3 {
            let (wr, wi) = (w[j].x as f64 * s, w[j].y as f64 * s);
            match pinned(t[j]) {
                Some(uv) => {
                    b_real -= wr * uv.x as f64 - wi * uv.y as f64;
                    b_imag -= wi * uv.x as f64 + wr * uv.y as f64;
                }
                None => {
                    let c = column[t[j]];
                    real.push((c, wr)); real.push((free + c, -wi));
                    imag.push((c, wi)); imag.push((free + c, wr));
                }
            }
        }
        rows.push(real); rhs.push(b_real);
        rows.push(imag); rhs.push(b_imag);
    }

    let x = least_squares(&rows, &rhs, free * 2);
    (0..n).map(|v| pinned(v).unwrap_or_else(|| Vec2::new(x[column[v]] as f32, x[free + column[v]] as f32))).collect()
}

/// Minimise |A x - b| by conjugate gradients on the normal equations
/// (CGLS), with a diagonal preconditioner.
fn least_squares(rows: &[Vec<(usize, f64)>], b: &[f64], n: usize) -> Vec<f64> {
    let apply = |x: &[f64]| -> Vec<f64> { rows.iter().map(|r| r.iter().map(|(c, a)| a * x[*c]).sum()).collect() };
    let apply_t = |y: &[f64]| -> Vec<f64> {
        let mut out = vec![0.0; n];
        for (r, yi) in rows.iter().zip(y) { for (c, a) in r { out[*c] += a * yi; } }
        out
    };
    let mut diag = vec![0.0f64; n];
    for r in rows { for (c, a) in r { diag[*c] += a * a; } }
    for d in &mut diag { *d = if *d > 1e-30 { 1.0 / *d } else { 1.0 }; }

    let mut x = vec![0.0; n];
    let mut r: Vec<f64> = b.to_vec();
    let mut s = apply_t(&r);
    let mut z: Vec<f64> = s.iter().zip(&diag).map(|(a, d)| a * d).collect();
    let mut p = z.clone();
    let mut gamma: f64 = s.iter().zip(&z).map(|(a, b)| a * b).sum();
    let start = gamma.max(1e-300);
    for _ in 0..(n * 4).clamp(50, 20_000) {
        if gamma <= start * 1e-18 { break; }
        let q = apply(&p);
        let qq: f64 = q.iter().map(|v| v * v).sum();
        if qq <= 1e-300 { break; }
        let alpha = gamma / qq;
        for i in 0..n { x[i] += alpha * p[i]; }
        for i in 0..r.len() { r[i] -= alpha * q[i]; }
        s = apply_t(&r);
        z = s.iter().zip(&diag).map(|(a, d)| a * d).collect();
        let next: f64 = s.iter().zip(&z).map(|(a, b)| a * b).sum();
        let beta = next / gamma;
        for i in 0..n { p[i] = z[i] + beta * p[i]; }
        gamma = next;
    }
    x
}

// ============================================================================
// UNWRAP
// ============================================================================

/// One flattened chart before packing.
struct Chart {
    tris: Vec<usize>,
    /// UV of each corner of each triangle, in `tris` order.
    uv:   Vec<[Vec2; 3]>,
}

impl Chart {
    fn bounds(&self) -> (Vec2, Vec2) {
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for t in &self.uv { for p in t { lo = lo.min(*p); hi = hi.max(*p); } }
        (lo, hi)
    }
}

fn uv_area(t: &[Vec2; 3]) -> f32 { (t[1] - t[0]).perp_dot(t[2] - t[0]) * 0.5 }

/// UVs for a mesh, one per triangle corner. `margin` is the gap between
/// charts as a fraction of the square.
pub fn unwrap(mesh: &MeshData, method: UvMethod, angle_deg: f32, margin: f32, axis: usize) -> Vec<[f32; 2]> {
    let nt = tri_count(mesh);
    if nt == 0 { return vec![]; }
    let mut charts: Vec<Chart> = vec![];
    match method {
        UvMethod::Planar => {
            let (u, v) = plane_axes(axis.min(2), 1.0);
            let uv = (0..nt).map(|t| tri(mesh, t).map(|i| Vec2::new(pos(mesh, i).dot(u), pos(mesh, i).dot(v)))).collect();
            charts.push(Chart { tris: (0..nt).collect(), uv });
        }
        UvMethod::Box => {
            // One chart per side of the box, by the triangle's dominant axis.
            let mut sides: Vec<Chart> = (0..6).map(|_| Chart { tris: vec![], uv: vec![] }).collect();
            for t in 0..nt {
                let n = tri_normal_area(mesh, t).0;
                let a = if n.x.abs() >= n.y.abs() && n.x.abs() >= n.z.abs() { 0 } else if n.y.abs() >= n.z.abs() { 1 } else { 2 };
                let sign = if n[a] < 0.0 { -1.0 } else { 1.0 };
                let (u, v) = plane_axes(a, sign);
                let side = &mut sides[a * 2 + (sign < 0.0) as usize];
                side.tris.push(t);
                side.uv.push(tri(mesh, t).map(|i| Vec2::new(pos(mesh, i).dot(u), pos(mesh, i).dot(v))));
            }
            charts = sides.into_iter().filter(|c| !c.tris.is_empty()).collect();
        }
        UvMethod::Conformal => {
            let (chart_of, count) = charts_by_angle(mesh, angle_deg);
            let mut members: Vec<Vec<usize>> = vec![vec![]; count];
            for t in 0..nt { members[chart_of[t]].push(t); }
            for tris in members {
                // Local numbering of the chart's vertices.
                let mut local: HashMap<u32, usize> = HashMap::new();
                let mut points = vec![];
                let faces: Vec<[usize; 3]> = tris.iter().map(|t| tri(mesh, *t).map(|v| {
                    *local.entry(v).or_insert_with(|| { points.push(pos(mesh, v)); points.len() - 1 })
                })).collect();
                let flat = lscm(&points, &faces);
                let mut uv: Vec<[Vec2; 3]> = faces.iter().map(|f| f.map(|i| flat[i])).collect();
                // Keep the chart the right way round, and at the scale of the surface.
                let area_uv: f32 = uv.iter().map(uv_area).sum();
                let area_3d: f32 = tris.iter().map(|t| tri_normal_area(mesh, *t).1).sum();
                let flip = if area_uv < 0.0 { -1.0 } else { 1.0 };
                let scale = if area_uv.abs() > 1e-12 { (area_3d / area_uv.abs()).sqrt() } else { 1.0 };
                if !scale.is_finite() { continue; }
                for t in &mut uv { for p in t.iter_mut() { *p = Vec2::new(p.x * flip, p.y) * scale; } }
                // Turn the chart so its bounding box is as small as it gets:
                // straight charts then pack straight.
                let box_area = |angle: f32| -> f32 {
                    let (sn, cs) = angle.sin_cos();
                    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
                    for t in &uv { for p in t { let q = Vec2::new(p.x * cs - p.y * sn, p.x * sn + p.y * cs); lo = lo.min(q); hi = hi.max(q); } }
                    (hi.x - lo.x) * (hi.y - lo.y)
                };
                let mut best = (box_area(0.0), 0.0f32);
                for k in 1..90 {
                    let a = (k as f32).to_radians();
                    let area = box_area(a);
                    if area < best.0 * 0.999 { best = (area, a); }
                }
                let (sn, cs) = best.1.sin_cos();
                for t in &mut uv { for p in t.iter_mut() { *p = Vec2::new(p.x * cs - p.y * sn, p.x * sn + p.y * cs); } }
                charts.push(Chart { tris, uv });
            }
        }
    }
    pack(&mut charts, margin);
    let mut out = vec![[0.0f32; 2]; nt * 3];
    for c in &charts {
        for (t, uv) in c.tris.iter().zip(&c.uv) {
            for k in 0..3 { out[t * 3 + k] = uv[k].to_array(); }
        }
    }
    out
}

/// In-plane axes for a projection along an axis, so the image is not mirrored.
fn plane_axes(axis: usize, sign: f32) -> (Vec3, Vec3) {
    match axis {
        0 => (Vec3::new(0.0, 0.0, -sign), Vec3::Y),
        1 => (Vec3::X, Vec3::new(0.0, 0.0, -sign)),
        _ => (Vec3::new(sign, 0.0, 0.0), Vec3::Y),
    }
}

/// Place the charts in the unit square, in rows, tallest first.
fn pack(charts: &mut [Chart], margin: f32) {
    if charts.is_empty() { return; }
    let sizes: Vec<Vec2> = charts.iter().map(|c| { let (lo, hi) = c.bounds(); (hi - lo).max(Vec2::splat(1e-6)) }).collect();
    let total: f32 = sizes.iter().map(|s| s.x * s.y).sum();
    let widest = sizes.iter().map(|s| s.x).fold(0.0, f32::max);
    let gap = margin.clamp(0.0, 0.2) * total.sqrt();
    let mut order: Vec<usize> = (0..charts.len()).collect();
    order.sort_by(|a, b| sizes[*b].y.total_cmp(&sizes[*a].y).then(a.cmp(b)));

    // Try a few row widths and keep the squarest result.
    let mut best: Option<(f32, Vec<Vec2>)> = None;
    for k in 0..24 {
        let width = (total.sqrt() * (1.0 + k as f32 * 0.08)).max(widest) + gap;
        let mut at = vec![Vec2::ZERO; charts.len()];
        let (mut x, mut y, mut row_h, mut used_w) = (gap, gap, 0.0f32, 0.0f32);
        for i in &order {
            if x + sizes[*i].x + gap > width && x > gap { x = gap; y += row_h + gap; row_h = 0.0; }
            at[*i] = Vec2::new(x, y);
            x += sizes[*i].x + gap;
            used_w = used_w.max(x);
            row_h = row_h.max(sizes[*i].y);
        }
        let side = used_w.max(y + row_h + gap);
        if best.as_ref().map(|b| side < b.0).unwrap_or(true) { best = Some((side, at)); }
    }
    let (side, at) = best.unwrap();
    for (i, c) in charts.iter_mut().enumerate() {
        let lo = c.bounds().0;
        for t in &mut c.uv { for p in t.iter_mut() { *p = (*p - lo + at[i]) / side; } }
    }
}

// ============================================================================
// ISLANDS AND EDITING
// ============================================================================

/// Islands of an existing UV layout: triangles joined through edges where
/// their UVs agree. Returns the island of each triangle and the count.
/// Islands are numbered by their first triangle.
pub fn islands(mesh: &MeshData) -> (Vec<usize>, usize) {
    let nt = tri_count(mesh);
    if mesh.uvs.len() != nt * 3 { return (vec![0; nt], (nt > 0) as usize); }
    let quant = |uv: [f32; 2]| ((uv[0] * 1.0e5).round() as i64, (uv[1] * 1.0e5).round() as i64);
    // An edge in UV space: both vertices and both UVs, in a fixed order.
    let mut by_edge: HashMap<((u32, (i64, i64)), (u32, (i64, i64))), Vec<usize>> = HashMap::new();
    for t in 0..nt {
        let v = tri(mesh, t);
        for k in 0..3 {
            let a = (v[k], quant(mesh.uvs[t * 3 + k]));
            let b = (v[(k + 1) % 3], quant(mesh.uvs[t * 3 + (k + 1) % 3]));
            by_edge.entry(if a <= b { (a, b) } else { (b, a) }).or_default().push(t);
        }
    }
    let mut parent: Vec<usize> = (0..nt).collect();
    fn find(p: &mut Vec<usize>, mut x: usize) -> usize { while p[x] != x { p[x] = p[p[x]]; x = p[x]; } x }
    for tris in by_edge.values() {
        for t in tris.iter().skip(1) {
            let (a, b) = (find(&mut parent, tris[0]), find(&mut parent, *t));
            if a != b { parent[a.max(b)] = a.min(b); }
        }
    }
    let mut id: HashMap<usize, usize> = HashMap::new();
    let out = (0..nt).map(|t| { let r = find(&mut parent, t); let n = id.len(); *id.entry(r).or_insert(n) }).collect();
    (out, id.len())
}

/// UV edges for drawing: every edge once, with a flag for seams (edges
/// used by one triangle only in UV space) and the first triangle on it.
pub fn uv_edges(mesh: &MeshData) -> Vec<([f32; 2], [f32; 2], bool, usize)> {
    let nt = tri_count(mesh);
    if mesh.uvs.len() != nt * 3 { return vec![]; }
    let quant = |uv: [f32; 2]| ((uv[0] * 1.0e5).round() as i64, (uv[1] * 1.0e5).round() as i64);
    let mut seen: HashMap<((i64, i64), (i64, i64)), (usize, u32)> = HashMap::new();
    let mut out: Vec<([f32; 2], [f32; 2], bool, usize)> = vec![];
    for t in 0..nt {
        for k in 0..3 {
            let (a, b) = (mesh.uvs[t * 3 + k], mesh.uvs[t * 3 + (k + 1) % 3]);
            let (qa, qb) = (quant(a), quant(b));
            let key = if qa <= qb { (qa, qb) } else { (qb, qa) };
            match seen.get_mut(&key) {
                Some((i, n)) => { *n += 1; out[*i].2 = false; }
                None => { seen.insert(key, (out.len(), 1)); out.push((a, b, true, t)); }
            }
        }
    }
    out
}

/// Island under a point in UV space.
pub fn island_at(mesh: &MeshData, island_of: &[usize], p: Vec2) -> Option<usize> {
    let nt = tri_count(mesh);
    if mesh.uvs.len() != nt * 3 { return None; }
    (0..nt).find(|t| {
        let [a, b, c] = [0, 1, 2].map(|k| Vec2::from_array(mesh.uvs[t * 3 + k]));
        let (d1, d2, d3) = ((b - a).perp_dot(p - a), (c - b).perp_dot(p - b), (a - c).perp_dot(p - c));
        (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
    }).map(|t| island_of[t])
}

/// Share of the unit square the UVs cover.
pub fn coverage(mesh: &MeshData) -> f32 {
    mesh.uvs.chunks_exact(3).map(|t| uv_area(&[Vec2::from_array(t[0]), Vec2::from_array(t[1]), Vec2::from_array(t[2])]).abs()).sum()
}

fn transform_about(uvs: &mut [[f32; 2]], corners: impl Iterator<Item = usize> + Clone, offset: Vec2, rotate_deg: f32, scale: Vec2) {
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for i in corners.clone() { let p = Vec2::from_array(uvs[i]); lo = lo.min(p); hi = hi.max(p); }
    if lo.x > hi.x { return; }
    let centre = (lo + hi) * 0.5;
    let (s, c) = rotate_deg.to_radians().sin_cos();
    for i in corners {
        let d = (Vec2::from_array(uvs[i]) - centre) * scale;
        uvs[i] = (centre + Vec2::new(d.x * c - d.y * s, d.x * s + d.y * c) + offset).to_array();
    }
}

/// Move, turn and scale the whole layout about its centre.
pub fn transform_all(mesh: &MeshData, offset: [f32; 2], rotate_deg: f32, scale: [f32; 2]) -> Vec<[f32; 2]> {
    let mut uvs = mesh.uvs.clone();
    transform_about(&mut uvs, 0..mesh.uvs.len(), Vec2::from_array(offset), rotate_deg, Vec2::from_array(scale));
    uvs
}

/// Apply island edits, in order. Islands are those of the incoming layout.
pub fn edit_islands(mesh: &MeshData, edits: &[IslandEdit]) -> Vec<[f32; 2]> {
    let mut uvs = mesh.uvs.clone();
    if edits.is_empty() || uvs.is_empty() { return uvs; }
    let (island_of, _) = islands(mesh);
    for e in edits {
        let corners: Vec<usize> = (0..island_of.len()).filter(|t| island_of[*t] == e.island as usize)
            .flat_map(|t| [t * 3, t * 3 + 1, t * 3 + 2]).collect();
        transform_about(&mut uvs, corners.iter().copied(), Vec2::from_array(e.offset), e.rotate, Vec2::from_array(e.scale));
    }
    uvs
}

// ── Remembered unwraps ───────────────────────────────────────────────────────

static CACHE: std::sync::Mutex<Vec<(u64, std::sync::Arc<Vec<[f32; 2]>>)>> = std::sync::Mutex::new(Vec::new());

/// `unwrap` with a memory: the same mesh and settings give the stored
/// result, so the several panes that cook a node do the work once.
pub fn unwrap_cached(mesh: &MeshData, method: UvMethod, angle_deg: f32, margin: f32, axis: usize) -> std::sync::Arc<Vec<[f32; 2]>> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for v in &mesh.vertices { for c in v { c.to_bits().hash(&mut h); } }
    mesh.indices.hash(&mut h);
    (method as u8, angle_deg.to_bits(), margin.to_bits(), axis).hash(&mut h);
    let key = h.finish();
    if let Ok(c) = CACHE.lock() {
        if let Some((_, uv)) = c.iter().find(|(k, _)| *k == key) { return uv.clone(); }
    }
    let uv = std::sync::Arc::new(unwrap(mesh, method, angle_deg, margin, axis));
    if let Ok(mut c) = CACHE.lock() {
        if c.len() >= 8 { c.remove(0); }
        c.push((key, uv.clone()));
    }
    uv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_graph::nodes::{create_cube, create_grid, create_sphere};

    fn with_uvs(mut m: MeshData, method: UvMethod, angle: f32) -> MeshData {
        m.uvs = unwrap(&m, method, angle, 0.02, 1);
        m
    }
    fn in_unit_square(m: &MeshData) -> bool {
        m.uvs.iter().all(|uv| uv[0] >= -1e-4 && uv[0] <= 1.0001 && uv[1] >= -1e-4 && uv[1] <= 1.0001)
    }
    /// Largest difference, over all triangles, between a corner angle in
    /// 3D and the same corner in UV space, in degrees.
    fn worst_angle_error(m: &MeshData) -> f32 {
        let mut worst = 0.0f32;
        for t in 0..tri_count(m) {
            let v = tri(m, t);
            if tri_normal_area(m, t).1 < 1e-9 { continue; }
            for k in 0..3 {
                let (a, b, c) = (pos(m, v[k]), pos(m, v[(k + 1) % 3]), pos(m, v[(k + 2) % 3]));
                let [ua, ub, uc] = [k, (k + 1) % 3, (k + 2) % 3].map(|j| Vec2::from_array(m.uvs[t * 3 + j]));
                let a3 = (b - a).angle_between(c - a);
                let a2 = (ub - ua).angle_between(uc - ua).abs();
                worst = worst.max((a3 - a2).abs().to_degrees());
            }
        }
        worst
    }

    #[test]
    fn a_flat_sheet_unwraps_without_distortion() {
        // A grid is already flat: LSCM must give it back, up to a similarity.
        let m = with_uvs(create_grid(6, 6, 3.0), UvMethod::Conformal, 66.0);
        assert_eq!(m.uvs.len(), m.indices.len());
        assert!(in_unit_square(&m));
        assert!(worst_angle_error(&m) < 0.05, "{}", worst_angle_error(&m));
        assert_eq!(islands(&m).1, 1);
        // It fills the square but for the margin.
        assert!(coverage(&m) > 0.9, "{}", coverage(&m));
        // Not mirrored: triangles wind the same way in UV as in 3D seen from above.
        let signed: f32 = m.uvs.chunks_exact(3).map(|t| uv_area(&[Vec2::from_array(t[0]), Vec2::from_array(t[1]), Vec2::from_array(t[2])])).sum();
        assert!(signed > 0.0);
    }

    #[test]
    fn a_bent_sheet_keeps_its_angles() {
        // A grid folded along a line: developable, so no distortion is needed.
        let mut m = create_grid(8, 8, 4.0);
        for v in &mut m.vertices { if v[0] > 0.0 { v[1] = v[0] * 0.6; } }
        let m = with_uvs(m, UvMethod::Conformal, 80.0);
        assert_eq!(islands(&m).1, 1);
        assert!(worst_angle_error(&m) < 0.5, "{}", worst_angle_error(&m));
    }

    #[test]
    fn a_cube_becomes_six_square_charts() {
        let m = with_uvs(create_cube(1.0), UvMethod::Conformal, 45.0);
        assert!(in_unit_square(&m));
        let (island_of, count) = islands(&m);
        assert_eq!(count, 6);
        assert!(worst_angle_error(&m) < 0.05);
        // Equal texel density: every face covers the same area.
        let mut area = vec![0.0f32; 6];
        for (t, uv) in m.uvs.chunks_exact(3).enumerate() {
            area[island_of[t]] += uv_area(&[Vec2::from_array(uv[0]), Vec2::from_array(uv[1]), Vec2::from_array(uv[2])]).abs();
        }
        for a in &area { assert!((a - area[0]).abs() < 1e-4, "{area:?}"); }
        // Charts do not overlap: total area is the sum of the parts and fits.
        assert!(coverage(&m) < 1.0 && coverage(&m) > 0.3);
        // Box projection gives the same six.
        let b = with_uvs(create_cube(1.0), UvMethod::Box, 0.0);
        assert_eq!(islands(&b).1, 6);
        assert!(in_unit_square(&b) && worst_angle_error(&b) < 0.05);
    }

    #[test]
    fn a_sphere_is_cut_into_charts_with_low_distortion() {
        let m = with_uvs(create_sphere(1.0, 24), UvMethod::Conformal, 50.0);
        assert!(in_unit_square(&m));
        let count = islands(&m).1;
        assert!(count >= 4 && count < 40, "{count} charts");
        assert!(m.uvs.iter().all(|uv| uv[0].is_finite() && uv[1].is_finite()));
        // Curved, so some distortion, but angles stay close.
        assert!(worst_angle_error(&m) < 25.0, "{}", worst_angle_error(&m));
        // A narrower angle gives more, flatter charts.
        let fine = with_uvs(create_sphere(1.0, 24), UvMethod::Conformal, 25.0);
        assert!(islands(&fine).1 > count);
        assert!(worst_angle_error(&fine) < worst_angle_error(&m));
    }

    #[test]
    fn planar_projects_along_an_axis() {
        let m = with_uvs(create_grid(2, 2, 2.0), UvMethod::Planar, 0.0);
        assert_eq!(islands(&m).1, 1);
        assert!(in_unit_square(&m) && worst_angle_error(&m) < 0.01);
        // A cube seen from above: the sides collapse to lines.
        let c = with_uvs(create_cube(1.0), UvMethod::Planar, 0.0);
        assert!(in_unit_square(&c));
        assert!(coverage(&c) > 0.8);
    }

    #[test]
    fn islands_can_be_moved_turned_and_scaled() {
        let m = with_uvs(create_cube(1.0), UvMethod::Conformal, 45.0);
        let (island_of, _) = islands(&m);
        let centre = |uvs: &[[f32; 2]], island: usize| -> Vec2 {
            let pts: Vec<Vec2> = (0..island_of.len()).filter(|t| island_of[*t] == island)
                .flat_map(|t| [0, 1, 2].map(|k| Vec2::from_array(uvs[t * 3 + k]))).collect();
            let (lo, hi) = pts.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(l, h), p| (l.min(*p), h.max(*p)));
            (lo + hi) * 0.5
        };
        let edit = IslandEdit { island: 2, offset: [0.25, -0.1], rotate: 90.0, scale: [2.0, 2.0] };
        let out = edit_islands(&m, &[edit]);
        // The island moved; the others did not.
        assert!((centre(&out, 2) - centre(&m.uvs, 2) - Vec2::new(0.25, -0.1)).length() < 1e-5);
        for i in [0, 1, 3, 4, 5] { assert!((centre(&out, i) - centre(&m.uvs, i)).length() < 1e-6); }
        // Twice the size: four times the area.
        let area = |uvs: &[[f32; 2]]| -> f32 {
            (0..island_of.len()).filter(|t| island_of[*t] == 2)
                .map(|t| uv_area(&[0, 1, 2].map(|k| Vec2::from_array(uvs[t * 3 + k]))).abs()).sum()
        };
        assert!((area(&out) / area(&m.uvs) - 4.0).abs() < 1e-3);
        // Picking: the middle of an island finds it.
        assert_eq!(island_at(&m, &island_of, centre(&m.uvs, 4)), Some(4));
        assert_eq!(island_at(&m, &island_of, Vec2::new(5.0, 5.0)), None);
        // Whole layout.
        let all = transform_all(&m, [0.5, 0.0], 0.0, [1.0, 1.0]);
        assert!((all[0][0] - m.uvs[0][0] - 0.5).abs() < 1e-6);
        // Seams: a cube's six squares have 24 seam edges and 6 diagonals inside.
        let edges = uv_edges(&m);
        assert_eq!(edges.iter().filter(|e| e.2).count(), 24);
        assert_eq!(edges.iter().filter(|e| !e.2).count(), 6);
    }
}
