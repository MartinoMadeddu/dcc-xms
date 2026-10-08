//! Maths of the viewport manipulators: where the handles are, which one is
//! under the cursor, and what a drag means. No Bevy systems here.

use bevy::math::{Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use super::poly::PickView;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tool { #[default] Select, Move, Rotate, Scale }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// X, Y or Z.
    Axis(usize),
    /// Move in the plane of the screen, or scale evenly.
    Centre,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Delta { Translate(Vec3), Rotate(Quat), Scale(Vec3) }

pub const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
/// Points along a rotation ring.
pub const RING_STEPS: usize = 48;

/// Length of the handles, so the manipulator keeps its size on screen.
pub fn gizmo_size(view: &PickView, pivot: Vec3) -> f32 {
    (view.eye.distance(pivot) * 0.16).max(1e-4)
}

/// Point `k` of the ring around `axis`.
pub fn ring_point(pivot: Vec3, size: f32, axis: usize, k: usize) -> Vec3 {
    let (u, v) = (AXES[(axis + 1) % 3], AXES[(axis + 2) % 3]);
    let a = k as f32 / RING_STEPS as f32 * std::f32::consts::TAU;
    pivot + (u * a.cos() + v * a.sin()) * size
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() < 1e-9 { 0.0 } else { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) };
    p.distance(a + ab * t)
}

/// Handle within `radius` pixels of the cursor.
pub fn hit_handle(view: &PickView, tool: Tool, pivot: Vec3, size: f32, pixel: Vec2, radius: f32) -> Option<Handle> {
    if tool == Tool::Select { return None; }
    let centre = view.project(pivot)?;
    if tool != Tool::Rotate && centre.distance(pixel) <= radius { return Some(Handle::Centre); }
    let mut best: Option<(usize, f32)> = None;
    for a in 0..3 {
        let d = match tool {
            Tool::Rotate => (0..RING_STEPS).filter_map(|k| {
                let p = view.project(ring_point(pivot, size, a, k))?;
                let q = view.project(ring_point(pivot, size, a, k + 1))?;
                Some(segment_distance(pixel, p, q))
            }).fold(f32::MAX, f32::min),
            _ => match view.project(pivot + AXES[a] * size) {
                Some(tip) => segment_distance(pixel, centre, tip),
                None => f32::MAX,
            },
        };
        if d <= radius && best.map(|b| d < b.1).unwrap_or(true) { best = Some((a, d)); }
    }
    best.map(|(a, _)| Handle::Axis(a))
}

/// Position along the line `p + t * dir` closest to a ray.
fn along_line(p: Vec3, dir: Vec3, o: Vec3, d: Vec3) -> Option<f32> {
    let w = p - o;
    let (b, e, f) = (dir.dot(d), dir.dot(w), d.dot(w));
    let den = 1.0 - b * b;
    (den.abs() > 1e-5).then(|| (b * f - e) / den)
}

fn on_plane(p: Vec3, n: Vec3, o: Vec3, d: Vec3) -> Option<Vec3> {
    let den = d.dot(n);
    if den.abs() < 1e-5 { return None; }
    let t = (p - o).dot(n) / den;
    (t > 0.0).then(|| o + d * t)
}

/// What dragging a handle from one pixel to another does. `pivot` and
/// `size` are those of the manipulator when the drag started.
pub fn drag_delta(view: &PickView, tool: Tool, handle: Handle, pivot: Vec3, size: f32, from: Vec2, to: Vec2) -> Delta {
    let (o0, d0) = view.ray(from);
    let (o1, d1) = view.ray(to);
    let facing = (pivot - view.eye).normalize_or_zero();
    match (tool, handle) {
        (Tool::Move, Handle::Axis(a)) => {
            let t = match (along_line(pivot, AXES[a], o0, d0), along_line(pivot, AXES[a], o1, d1)) {
                (Some(t0), Some(t1)) => t1 - t0,
                _ => 0.0,
            };
            Delta::Translate(AXES[a] * t)
        }
        (Tool::Move, Handle::Centre) => {
            match (on_plane(pivot, facing, o0, d0), on_plane(pivot, facing, o1, d1)) {
                (Some(a), Some(b)) => Delta::Translate(b - a),
                _ => Delta::Translate(Vec3::ZERO),
            }
        }
        (Tool::Rotate, Handle::Axis(a)) => {
            let axis = AXES[a];
            let angle = if facing.dot(axis).abs() > 0.15 {
                match (on_plane(pivot, axis, o0, d0), on_plane(pivot, axis, o1, d1)) {
                    (Some(p0), Some(p1)) => {
                        let (u, v) = ((p0 - pivot).normalize_or_zero(), (p1 - pivot).normalize_or_zero());
                        u.cross(v).dot(axis).atan2(u.dot(v))
                    }
                    _ => 0.0,
                }
            } else {
                // Ring seen edge-on: the drag along it sets the angle.
                (to - from).length() * 0.01 * if (to - from).dot(Vec2::new(1.0, -1.0)) < 0.0 { -1.0 } else { 1.0 }
            };
            Delta::Rotate(Quat::from_axis_angle(axis, angle))
        }
        (Tool::Scale, Handle::Axis(a)) => {
            let f = match (along_line(pivot, AXES[a], o0, d0), along_line(pivot, AXES[a], o1, d1)) {
                (Some(t0), Some(t1)) if t0.abs() > size * 0.02 => (t1 / t0).clamp(-100.0, 100.0),
                _ => 1.0,
            };
            let mut s = Vec3::ONE;
            s[a] = f;
            Delta::Scale(s)
        }
        (Tool::Scale, Handle::Centre) => {
            let px = (to.x - from.x) - (to.y - from.y);
            Delta::Scale(Vec3::splat((1.0 + px * 0.005).max(0.01)))
        }
        _ => Delta::Translate(Vec3::ZERO),
    }
}

/// A drag applied on top of the transform an operation had when it began.
/// Returns translate, rotate, scale.
pub fn compose(base: (Vec3, Quat, Vec3), delta: Delta) -> (Vec3, Quat, Vec3) {
    let (t, r, s) = base;
    match delta {
        Delta::Translate(d) => (t + d, r, s),
        Delta::Rotate(q)    => (t, (q * r).normalize(), s),
        Delta::Scale(k)     => (t, r, s * k),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Mat4;

    fn camera(eye: Vec3) -> PickView {
        let size = Vec2::new(800.0, 600.0);
        let proj = Mat4::perspective_infinite_reverse_rh(0.8, size.x / size.y, 0.1);
        PickView { view_proj: proj * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y), size, eye }
    }

    #[test]
    fn handles_are_found_under_the_cursor() {
        let view = camera(Vec3::new(3.0, 2.5, 5.0));
        let pivot = Vec3::ZERO;
        let size = gizmo_size(&view, pivot);
        for a in 0..3 {
            let mid = view.project(pivot + AXES[a] * size * 0.7).unwrap();
            assert_eq!(hit_handle(&view, Tool::Move, pivot, size, mid, 8.0), Some(Handle::Axis(a)));
            assert_eq!(hit_handle(&view, Tool::Scale, pivot, size, mid, 8.0), Some(Handle::Axis(a)));
            let on_ring = view.project(ring_point(pivot, size, a, 5)).unwrap();
            assert_eq!(hit_handle(&view, Tool::Rotate, pivot, size, on_ring, 6.0), Some(Handle::Axis(a)));
        }
        let c = view.project(pivot).unwrap();
        assert_eq!(hit_handle(&view, Tool::Move, pivot, size, c, 8.0), Some(Handle::Centre));
        assert_eq!(hit_handle(&view, Tool::Move, pivot, size, c + Vec2::new(300.0, 250.0), 8.0), None);
        assert_eq!(hit_handle(&view, Tool::Select, pivot, size, c, 8.0), None);
    }

    #[test]
    fn dragging_an_axis_moves_along_it() {
        let view = camera(Vec3::new(3.0, 2.5, 5.0));
        let pivot = Vec3::new(0.2, 0.1, -0.3);
        let size = gizmo_size(&view, pivot);
        for a in 0..3 {
            let from = view.project(pivot + AXES[a] * 0.5).unwrap();
            let to   = view.project(pivot + AXES[a] * 1.25).unwrap();
            let Delta::Translate(d) = drag_delta(&view, Tool::Move, Handle::Axis(a), pivot, size, from, to) else { panic!() };
            assert!((d - AXES[a] * 0.75).length() < 2e-3, "{a}: {d}");
            let Delta::Scale(s) = drag_delta(&view, Tool::Scale, Handle::Axis(a), pivot, size, from, to) else { panic!() };
            let mut want = Vec3::ONE;
            want[a] = 2.5;
            assert!((s - want).length() < 5e-3, "{a}: {s}");
        }
        // Free move stays in the plane facing the camera and follows the cursor.
        let target = pivot + Vec3::new(0.4, 0.3, 0.0);
        let (from, to) = (view.project(pivot).unwrap(), view.project(target).unwrap());
        let Delta::Translate(d) = drag_delta(&view, Tool::Move, Handle::Centre, pivot, size, from, to) else { panic!() };
        assert!(d.dot((pivot - view.eye).normalize()).abs() < 1e-3);
        assert!(view.project(pivot + d).unwrap().distance(to) < 0.5);
    }

    #[test]
    fn dragging_a_ring_turns_about_its_axis() {
        let view = camera(Vec3::new(3.0, 2.5, 5.0));
        let pivot = Vec3::ZERO;
        let size = gizmo_size(&view, pivot);
        for a in 0..3 {
            // An eighth of a turn along the ring.
            let from = view.project(ring_point(pivot, size, a, 2)).unwrap();
            let to   = view.project(ring_point(pivot, size, a, 2 + RING_STEPS / 8)).unwrap();
            let Delta::Rotate(q) = drag_delta(&view, Tool::Rotate, Handle::Axis(a), pivot, size, from, to) else { panic!() };
            let want = Quat::from_axis_angle(AXES[a], std::f32::consts::FRAC_PI_4);
            assert!(q.angle_between(want) < 5e-3, "{a}");
        }
    }

    #[test]
    fn drags_stack_on_the_starting_transform() {
        let base = (Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY, Vec3::new(2.0, 1.0, 1.0));
        assert_eq!(compose(base, Delta::Translate(Vec3::Y)).0, Vec3::new(1.0, 1.0, 0.0));
        assert_eq!(compose(base, Delta::Scale(Vec3::new(1.5, 1.0, 2.0))).2, Vec3::new(3.0, 1.0, 2.0));
        let q = Quat::from_rotation_y(0.5);
        let (t, r, s) = compose(base, Delta::Rotate(q));
        assert!(r.angle_between(q) < 1e-5 && t == base.0 && s == base.2);
    }
}
