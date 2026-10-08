//! A tree over the triangles of a collider, for the two questions the
//! solver asks: what is the nearest surface to this point, and does this
//! segment cross a surface. Any triangle soup will do: open, non-manifold,
//! two-sided, millions of triangles.

use glam::Vec3;

#[derive(Clone, Copy)]
struct Tri { a: Vec3, b: Vec3, c: Vec3 }

#[derive(Clone, Copy)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// Leaf: first triangle. Inner: left child (the right one follows it).
    first: u32,
    /// Triangles in a leaf, zero for an inner node.
    count: u32,
}

pub struct Bvh {
    nodes: Vec<Node>,
    tris:  Vec<Tri>,
}

#[derive(Clone, Copy, Debug)]
pub struct Nearest {
    pub point:  Vec3,
    pub dist:   f32,
    /// Normal of the triangle, by its winding.
    pub normal: Vec3,
}

#[derive(Clone, Copy, Debug)]
pub struct Crossing {
    /// Position along the segment, 0 at its start.
    pub t:      f32,
    pub point:  Vec3,
    pub normal: Vec3,
}

const LEAF: usize = 4;

impl Bvh {
    /// Build from positions and triangle indices. Triangles without area
    /// are left out.
    pub fn new(vertices: &[[f32; 3]], indices: &[u32]) -> Bvh {
        let v = |i: u32| vertices.get(i as usize).map(|p| Vec3::from_array(*p));
        let mut tris: Vec<Tri> = Vec::with_capacity(indices.len() / 3);
        for t in indices.chunks_exact(3) {
            let (Some(a), Some(b), Some(c)) = (v(t[0]), v(t[1]), v(t[2])) else { continue };
            if !(a.is_finite() && b.is_finite() && c.is_finite()) { continue; }
            if (b - a).cross(c - a).length_squared() < 1e-18 { continue; }
            tris.push(Tri { a, b, c });
        }
        let mut bvh = Bvh { nodes: Vec::with_capacity(tris.len() / 2 + 2), tris };
        if bvh.tris.is_empty() { return bvh; }
        let cent: Vec<Vec3> = bvh.tris.iter().map(|t| (t.a + t.b + t.c) / 3.0).collect();
        let mut order: Vec<u32> = (0..bvh.tris.len() as u32).collect();
        bvh.nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, first: 0, count: 0 });
        // (node, range) still to split.
        let mut todo = vec![(0usize, 0usize, order.len())];
        while let Some((ni, lo, hi)) = todo.pop() {
            let (mut mn, mut mx) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let (mut cmn, mut cmx) = (mn, mx);
            for &i in &order[lo..hi] {
                let t = &bvh.tris[i as usize];
                mn = mn.min(t.a).min(t.b).min(t.c);
                mx = mx.max(t.a).max(t.b).max(t.c);
                cmn = cmn.min(cent[i as usize]);
                cmx = cmx.max(cent[i as usize]);
            }
            bvh.nodes[ni].min = mn;
            bvh.nodes[ni].max = mx;
            let n = hi - lo;
            let ext = cmx - cmn;
            if n <= LEAF || ext.max_element() < 1e-9 {
                bvh.nodes[ni].first = lo as u32;
                bvh.nodes[ni].count = n as u32;
                continue;
            }
            let axis = if ext.x >= ext.y && ext.x >= ext.z { 0 } else if ext.y >= ext.z { 1 } else { 2 };
            let mid = n / 2;
            order[lo..hi].select_nth_unstable_by(mid, |a, b| {
                cent[*a as usize][axis].partial_cmp(&cent[*b as usize][axis]).unwrap_or(std::cmp::Ordering::Equal)
            });
            let left = bvh.nodes.len();
            bvh.nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, first: 0, count: 0 });
            bvh.nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, first: 0, count: 0 });
            bvh.nodes[ni].first = left as u32;
            bvh.nodes[ni].count = 0;
            todo.push((left, lo, lo + mid));
            todo.push((left + 1, lo + mid, hi));
        }
        // Triangles in leaf order, so a leaf reads memory in one run.
        bvh.tris = order.iter().map(|i| bvh.tris[*i as usize]).collect();
        bvh
    }

    pub fn len(&self) -> usize { self.tris.len() }
    pub fn is_empty(&self) -> bool { self.tris.is_empty() }

    /// Bounds of everything, or None when there are no triangles.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        if self.tris.is_empty() { None } else { Some((self.nodes[0].min, self.nodes[0].max)) }
    }

    /// Is any part of the collider within `radius` of `center`? Cheap and
    /// a little generous: it answers for the boxes of the leaves.
    pub fn near_sphere(&self, center: Vec3, radius: f32) -> bool {
        if self.tris.is_empty() { return false; }
        let r2 = radius * radius;
        let mut stack = [0u32; 64];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let n = &self.nodes[stack[sp] as usize];
            if box_dist2(n.min, n.max, center) > r2 { continue; }
            if n.count > 0 { return true; }
            if sp + 2 <= stack.len() { stack[sp] = n.first; stack[sp + 1] = n.first + 1; sp += 2; }
        }
        false
    }

    /// Nearest point of the collider within `radius` of `p`.
    pub fn closest(&self, p: Vec3, radius: f32) -> Option<Nearest> {
        if self.tris.is_empty() { return None; }
        let mut best2 = radius * radius;
        let mut best: Option<(Vec3, usize)> = None;
        let mut stack = [(0u32, 0f32); 64];
        let mut sp = 1;
        stack[0] = (0, box_dist2(self.nodes[0].min, self.nodes[0].max, p));
        while sp > 0 {
            sp -= 1;
            let (ni, d2) = stack[sp];
            if d2 > best2 { continue; }
            let n = &self.nodes[ni as usize];
            if n.count > 0 {
                for i in n.first as usize..(n.first + n.count) as usize {
                    let t = &self.tris[i];
                    let c = closest_on_triangle(p, t.a, t.b, t.c);
                    let d = (p - c).length_squared();
                    if d < best2 { best2 = d; best = Some((c, i)); }
                }
            } else if sp + 2 <= stack.len() {
                let (l, r) = (n.first, n.first + 1);
                let dl = box_dist2(self.nodes[l as usize].min, self.nodes[l as usize].max, p);
                let dr = box_dist2(self.nodes[r as usize].min, self.nodes[r as usize].max, p);
                // The nearer child is looked at first, so it goes on last.
                if dl < dr { stack[sp] = (r, dr); stack[sp + 1] = (l, dl); }
                else       { stack[sp] = (l, dl); stack[sp + 1] = (r, dr); }
                sp += 2;
            }
        }
        best.map(|(point, i)| {
            let t = &self.tris[i];
            Nearest { point, dist: best2.sqrt(), normal: (t.b - t.a).cross(t.c - t.a).normalize_or_zero() }
        })
    }

    /// First triangle the segment from `a` to `b` crosses, seen from `a`.
    pub fn segment(&self, a: Vec3, b: Vec3) -> Option<Crossing> {
        if self.tris.is_empty() { return None; }
        let d = b - a;
        let inv = Vec3::new(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
        let mut best_t = 1.0f32;
        let mut best: Option<usize> = None;
        let mut stack = [0u32; 64];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let n = &self.nodes[stack[sp] as usize];
            if !segment_hits_box(a, d, inv, n.min, n.max, best_t) { continue; }
            if n.count > 0 {
                for i in n.first as usize..(n.first + n.count) as usize {
                    let t = &self.tris[i];
                    if let Some(u) = segment_triangle(a, d, t.a, t.b, t.c) {
                        if u < best_t { best_t = u; best = Some(i); }
                    }
                }
            } else if sp + 2 <= stack.len() {
                stack[sp] = n.first; stack[sp + 1] = n.first + 1; sp += 2;
            }
        }
        best.map(|i| {
            let t = &self.tris[i];
            Crossing { t: best_t, point: a + d * best_t, normal: (t.b - t.a).cross(t.c - t.a).normalize_or_zero() }
        })
    }
}

fn box_dist2(min: Vec3, max: Vec3, p: Vec3) -> f32 {
    let d = (min - p).max(p - max).max(Vec3::ZERO);
    d.length_squared()
}

fn segment_hits_box(a: Vec3, d: Vec3, inv: Vec3, min: Vec3, max: Vec3, t_max: f32) -> bool {
    let (mut t0, mut t1) = (0.0f32, t_max);
    for k in 0..3 {
        if d[k].abs() < 1e-12 {
            if a[k] < min[k] || a[k] > max[k] { return false; }
        } else {
            let (mut u, mut v) = ((min[k] - a[k]) * inv[k], (max[k] - a[k]) * inv[k]);
            if u > v { std::mem::swap(&mut u, &mut v); }
            t0 = t0.max(u);
            t1 = t1.min(v);
            if t0 > t1 { return false; }
        }
    }
    true
}

/// Where along `a + d t`, t in 0..1, the segment crosses the triangle.
fn segment_triangle(a: Vec3, d: Vec3, p0: Vec3, p1: Vec3, p2: Vec3) -> Option<f32> {
    let (e1, e2) = (p1 - p0, p2 - p0);
    let h = d.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-12 { return None; }
    let f = 1.0 / det;
    let s = a - p0;
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) { return None; }
    let q = s.cross(e1);
    let v = f * d.dot(q);
    if v < 0.0 || u + v > 1.0 { return None; }
    let t = f * e2.dot(q);
    (0.0..=1.0).contains(&t).then_some(t)
}

/// Nearest point of a triangle to `p` (Ericson, Real-Time Collision Detection).
pub fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 { return a; }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 { return b; }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 { return a + ab * (d1 / (d1 - d3)); }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 { return c; }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 { return a + ac * (d2 / (d2 - d6)); }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid of n x n quads in the XZ plane at height y, facing up.
    pub fn floor(n: usize, size: f32, y: f32) -> (Vec<[f32; 3]>, Vec<u32>) {
        let mut v = vec![];
        let mut idx = vec![];
        for j in 0..=n { for i in 0..=n {
            v.push([(i as f32 / n as f32 - 0.5) * size, y, (j as f32 / n as f32 - 0.5) * size]);
        } }
        let w = (n + 1) as u32;
        for j in 0..n as u32 { for i in 0..n as u32 {
            let a = j * w + i;
            idx.extend([a, a + w, a + 1, a + 1, a + w, a + w + 1]);
        } }
        (v, idx)
    }

    #[test]
    fn nearest_matches_a_search_of_every_triangle() {
        // A bumpy sheet, so nearest points fall on faces, edges and corners.
        let (mut v, idx) = floor(40, 4.0, 0.0);
        for p in v.iter_mut() { p[1] = 0.3 * (p[0] * 3.0).sin() * (p[2] * 2.0).cos(); }
        let bvh = Bvh::new(&v, &idx);
        assert_eq!(bvh.len(), 40 * 40 * 2);
        let mut seed = 7u32;
        let mut rnd = || { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); (seed >> 8) as f32 / (1u32 << 24) as f32 };
        for _ in 0..300 {
            let p = Vec3::new(rnd() * 5.0 - 2.5, rnd() * 2.0 - 1.0, rnd() * 5.0 - 2.5);
            let mut want = f32::MAX;
            for t in idx.chunks_exact(3) {
                let c = closest_on_triangle(p, Vec3::from_array(v[t[0] as usize]), Vec3::from_array(v[t[1] as usize]), Vec3::from_array(v[t[2] as usize]));
                want = want.min((p - c).length());
            }
            let got = bvh.closest(p, 10.0).unwrap();
            assert!((got.dist - want).abs() < 1e-4, "{} against {}", got.dist, want);
            // Out of reach is nothing.
            assert!(bvh.closest(p, want * 0.99).is_none());
            assert!(bvh.near_sphere(p, want * 1.01));
        }
    }

    #[test]
    fn a_segment_finds_the_first_surface_it_crosses() {
        // Two floors, one above the other.
        let (mut v, mut idx) = floor(8, 2.0, 0.0);
        let (v2, idx2) = floor(8, 2.0, 1.0);
        let off = v.len() as u32;
        v.extend(v2);
        idx.extend(idx2.iter().map(|i| i + off));
        let bvh = Bvh::new(&v, &idx);
        let down = bvh.segment(Vec3::new(0.1, 2.0, 0.2), Vec3::new(0.1, -1.0, 0.2)).unwrap();
        assert!((down.point.y - 1.0).abs() < 1e-5 && (down.t - 1.0 / 3.0).abs() < 1e-5);
        let up = bvh.segment(Vec3::new(0.1, -1.0, 0.2), Vec3::new(0.1, 2.0, 0.2)).unwrap();
        assert!(up.point.y.abs() < 1e-5);
        assert!(up.normal.y.abs() > 0.999);
        // Between the two, and past the edge, nothing is crossed.
        assert!(bvh.segment(Vec3::new(0.0, 0.2, 0.0), Vec3::new(0.5, 0.8, 0.3)).is_none());
        assert!(bvh.segment(Vec3::new(3.0, 2.0, 0.0), Vec3::new(3.0, -1.0, 0.0)).is_none());
    }

    #[test]
    fn nothing_in_is_nothing_out() {
        let bvh = Bvh::new(&[], &[]);
        assert!(bvh.is_empty() && bvh.bounds().is_none());
        assert!(bvh.closest(Vec3::ZERO, 1.0).is_none() && bvh.segment(Vec3::ZERO, Vec3::ONE).is_none());
        // A triangle with no area is dropped.
        assert!(Bvh::new(&[[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]], &[0, 1, 2]).is_empty());
    }
}
