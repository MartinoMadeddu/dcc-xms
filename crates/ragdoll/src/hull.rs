//! Convex hull of one body segment.
//!
//! The hull is kept as a set of planes: for each of a fixed set of
//! directions spread evenly over the sphere, how far the segment reaches
//! that way. With 162 or 642 directions that is a tight convex wrap of the
//! skin, it is quick to test a point against, and it cannot fail on thin
//! or flat input the way an incremental hull can.
//!
//! The surface is also sampled, one point per direction, by walking out
//! from the middle until the first plane is met. Those points are what
//! touches a collider, and with the triangles of the sphere they are a
//! mesh to look at.

use std::sync::{Arc, OnceLock};
use glam::Vec3;

/// Directions and triangles of an icosahedron subdivided `level` times:
/// 12, 42, 162, 642 directions.
pub fn sphere(level: u8) -> (Arc<[Vec3]>, Arc<[[u32; 3]]>) {
    static CACHE: [OnceLock<(Arc<[Vec3]>, Arc<[[u32; 3]]>)>; 4] = [OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let level = level.min(3);
    CACHE[level as usize].get_or_init(|| {
        let t = (1.0 + 5.0f32.sqrt()) / 2.0;
        let mut v: Vec<Vec3> = [
            [-1.0, t, 0.0], [1.0, t, 0.0], [-1.0, -t, 0.0], [1.0, -t, 0.0],
            [0.0, -1.0, t], [0.0, 1.0, t], [0.0, -1.0, -t], [0.0, 1.0, -t],
            [t, 0.0, -1.0], [t, 0.0, 1.0], [-t, 0.0, -1.0], [-t, 0.0, 1.0],
        ].iter().map(|p| Vec3::from_array(*p).normalize()).collect();
        let mut f: Vec<[u32; 3]> = vec![
            [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
            [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
            [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
            [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
        ];
        for _ in 0..level {
            let mut mid = std::collections::HashMap::new();
            let mut next = Vec::with_capacity(f.len() * 4);
            for t in &f {
                let mut m = [0u32; 3];
                for k in 0..3 {
                    let (a, b) = (t[k], t[(k + 1) % 3]);
                    let key = (a.min(b), a.max(b));
                    m[k] = *mid.entry(key).or_insert_with(|| {
                        v.push(((v[a as usize] + v[b as usize]) * 0.5).normalize());
                        v.len() as u32 - 1
                    });
                }
                next.extend([[t[0], m[0], m[2]], [t[1], m[1], m[0]], [t[2], m[2], m[1]], [m[0], m[1], m[2]]]);
            }
            f = next;
        }
        (v.into(), f.into())
    }).clone()
}

#[derive(Clone)]
pub struct Hull {
    /// Unit directions, shared by every hull of the same resolution.
    pub dirs:    Arc<[Vec3]>,
    /// Reach along each direction: the hull is every x with dir . x <= reach.
    pub reach:   Vec<f32>,
    /// One point of the surface per direction.
    pub samples: Vec<Vec3>,
    /// Triangles over `samples`.
    pub tris:    Arc<[[u32; 3]]>,
    /// A point well inside.
    pub center:  Vec3,
    /// Everything is within this distance of `center`.
    pub radius:  f32,
    pub volume:  f32,
    /// Centre of mass, for an even density.
    pub com:     Vec3,
    /// Extent of the hull along the three axes of its space, through `com`.
    pub size:    Vec3,
}

impl Hull {
    /// Hull of a cloud of points. `level` 1, 2 or 3 gives 42, 162 or 642
    /// planes. Fewer than four points, or points in a plane or a line, are
    /// given a little thickness so the hull has an inside.
    pub fn from_points(points: &[Vec3], level: u8) -> Hull {
        let (dirs, tris) = sphere(level);
        let mut mean = Vec3::ZERO;
        for p in points { mean += *p; }
        if !points.is_empty() { mean /= points.len() as f32; }
        const MIN_HALF: f32 = 0.004;
        let reach: Vec<f32> = dirs.iter().map(|d| {
            let far = points.iter().map(|p| d.dot(*p)).fold(f32::MIN, f32::max);
            if points.is_empty() { MIN_HALF } else { far.max(d.dot(mean) + MIN_HALF) }
        }).collect();
        Hull::from_planes(dirs, tris, reach, mean)
    }

    /// A capsule from `a` to `b`, for a bone with no skin of its own.
    pub fn capsule(a: Vec3, b: Vec3, radius: f32, level: u8) -> Hull {
        let (dirs, tris) = sphere(level);
        let reach: Vec<f32> = dirs.iter().map(|d| d.dot(a).max(d.dot(b)) + radius).collect();
        Hull::from_planes(dirs, tris, reach, (a + b) * 0.5)
    }

    fn from_planes(dirs: Arc<[Vec3]>, tris: Arc<[[u32; 3]]>, reach: Vec<f32>, inside: Vec3) -> Hull {
        let n = dirs.len();
        // Walk out from the inside point along each direction to the first plane.
        let slack: Vec<f32> = (0..n).map(|j| reach[j] - dirs[j].dot(inside)).collect();
        let samples: Vec<Vec3> = (0..n).map(|i| {
            let mut t = f32::MAX;
            for j in 0..n {
                let c = dirs[i].dot(dirs[j]);
                if c > 1e-4 { t = t.min(slack[j] / c); }
            }
            inside + dirs[i] * t.max(0.0)
        }).collect();
        // Volume and centre of mass from the tetrahedra to the inside point.
        let (mut vol, mut com) = (0.0f32, Vec3::ZERO);
        for t in tris.iter() {
            let (a, b, c) = (samples[t[0] as usize] - inside, samples[t[1] as usize] - inside, samples[t[2] as usize] - inside);
            let v = a.dot(b.cross(c)) / 6.0;
            vol += v;
            com += (a + b + c) * (0.25 * v);
        }
        let vol_abs = vol.abs().max(1e-9);
        let com = inside + com / vol.abs().max(1e-12) * vol.signum();
        let com = if com.is_finite() { com } else { inside };
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let mut radius = 0.0f32;
        for s in &samples { lo = lo.min(*s); hi = hi.max(*s); radius = radius.max((*s - com).length()); }
        Hull { dirs, reach, samples, tris, center: com, radius, volume: vol_abs, com, size: (hi - lo).max(Vec3::splat(1e-3)) }
    }

    /// How far `p` is outside the hull: negative inside. With it, the
    /// direction of the plane that decides.
    pub fn signed(&self, p: Vec3) -> (f32, Vec3) {
        let mut best = f32::MIN;
        let mut at = 0;
        for (i, d) in self.dirs.iter().enumerate() {
            let s = d.dot(p) - self.reach[i];
            if s > best { best = s; at = i; }
        }
        (best, self.dirs[at])
    }

    /// Is `p` inside the hull by more than `allow`? Then how deep, and the
    /// direction of the plane it is nearest to. Stops at the first plane
    /// that says no, which for a point outside is one of the first few:
    /// the directions are stored coarse to fine.
    pub fn deeper_than(&self, p: Vec3, allow: f32) -> Option<(f32, Vec3)> {
        let mut best = f32::MIN;
        let mut at = 0;
        for (i, d) in self.dirs.iter().enumerate() {
            let s = d.dot(p) - self.reach[i];
            if s > -allow { return None; }
            if s > best { best = s; at = i; }
        }
        Some((-best, self.dirs[at]))
    }

    /// Moment of inertia about the three axes, for a mass, taking the hull
    /// for a box of its size.
    pub fn inertia(&self, mass: f32) -> Vec3 {
        let s = self.size;
        Vec3::new(s.y * s.y + s.z * s.z, s.x * s.x + s.z * s.z, s.x * s.x + s.y * s.y) * (mass / 12.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spheres_have_the_expected_counts() {
        for (level, n) in [(0u8, 12usize), (1, 42), (2, 162), (3, 642)] {
            let (d, t) = sphere(level);
            assert_eq!(d.len(), n);
            assert_eq!(t.len(), 20 * 4usize.pow(level as u32));
            assert!(d.iter().all(|v| (v.length() - 1.0).abs() < 1e-5));
        }
    }

    #[test]
    fn hull_of_a_box_is_the_box() {
        let mut pts = vec![];
        for x in [-0.3f32, 0.3] { for y in [-0.1f32, 0.1] { for z in [-0.2f32, 0.2] { pts.push(Vec3::new(x, y, z) + Vec3::new(1.0, 2.0, 3.0)); } } }
        let h = Hull::from_points(&pts, 2);
        let c = Vec3::new(1.0, 2.0, 3.0);
        // The corners are on it, the middle is inside, a point beyond a face is outside by that much.
        for p in &pts { assert!(h.signed(*p).0.abs() < 1e-4); }
        assert!(h.signed(c).0 < -0.09);
        // Planes cut the corners a little short of the box, never more than a few percent of it.
        assert!((h.volume - 0.6 * 0.2 * 0.4).abs() < 0.6 * 0.2 * 0.4 * 0.12, "{}", h.volume);
        assert!((h.com - c).length() < 0.01);
        assert!((h.size - Vec3::new(0.6, 0.2, 0.4)).abs().max_element() < 0.02);
        // Every sample is on the surface and inside the box.
        for s in &h.samples {
            assert!(h.signed(*s).0.abs() < 1e-4);
            let d = (*s - c).abs();
            assert!(d.x < 0.3001 && d.y < 0.1001 && d.z < 0.2001);
        }
        // Flat sides are sampled too, not only the corners.
        assert!(h.samples.iter().any(|s| ((*s - c).x.abs() < 0.05) && ((*s - c).z.abs() < 0.05)));
    }

    #[test]
    fn a_capsule_and_thin_input_have_an_inside() {
        let c = Hull::capsule(Vec3::ZERO, Vec3::new(0.0, 0.5, 0.0), 0.1, 2);
        let want = std::f32::consts::PI * 0.01 * 0.5 + 4.0 / 3.0 * std::f32::consts::PI * 0.001;
        assert!((c.volume - want).abs() < want * 0.15, "{} against {}", c.volume, want);
        assert!(c.signed(Vec3::new(0.0, 0.25, 0.0)).0 < -0.09);
        assert!((c.signed(Vec3::new(0.3, 0.25, 0.0)).0 - 0.2).abs() < 0.01);
        // Three points in a line still make a hull with volume.
        let line = Hull::from_points(&[Vec3::ZERO, Vec3::X * 0.5, Vec3::X], 1);
        assert!(line.volume > 0.0 && line.signed(Vec3::X * 0.5).0 < 0.0);
        let none = Hull::from_points(&[], 1);
        assert!(none.volume > 0.0);
    }
}
