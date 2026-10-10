use bevy::prelude::*;
use bevy_egui::EguiContexts;
use crate::types::{MainCamera, GeneratedMesh, ViewportRect};
use super::nav::{Held, NavAction, NavSettings, NavStyle};

#[derive(Resource, Default)]
pub struct CameraOrbitState {
    pub target: Vec3,
}

/// Keys and buttons as the navigation styles see them.
pub fn held(keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> Held {
    Held {
        alt:   keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight, KeyCode::SuperLeft, KeyCode::SuperRight]),
        shift: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        ctrl:  keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]),
        space: keys.pressed(KeyCode::Space),
        s:     keys.pressed(KeyCode::KeyS),
        lmb:   mouse.pressed(MouseButton::Left),
        mmb:   mouse.pressed(MouseButton::Middle),
        rmb:   mouse.pressed(MouseButton::Right),
    }
}

/// State of the camera drag between frames.
#[derive(Default)]
pub struct NavDrag {
    /// The drag began inside the viewport.
    active:     bool,
    /// Space was used to navigate since it went down (Houdini style).
    space_used: bool,
    /// Cursor position at the end of the last frame, in window pixels.
    last:       Option<Vec2>,
}

/// Orbit, pan and zoom, with the keys of the chosen navigation style.
pub fn camera_controller(
    mouse_btn:        Res<ButtonInput<MouseButton>>,
    mut cursor_moved: EventReader<CursorMoved>,
    mut mouse_wheel:  EventReader<bevy::input::mouse::MouseWheel>,
    keyboard:         Res<ButtonInput<KeyCode>>,
    nav:              Res<NavSettings>,
    vp_rect:          Res<ViewportRect>,
    windows:          Query<&Window>,
    mut cam_q:        Query<&mut Transform, With<MainCamera>>,
    mut orbit:        ResMut<CameraOrbitState>,
    mut playback:     ResMut<crate::timeline::Playback>,
    mut contexts:     EguiContexts,
    mut drag:         Local<NavDrag>,
) {
    let ctx = contexts.ctx_mut();
    let typing = ctx.wants_keyboard_input();
    let mut h = held(&keyboard, &mouse_btn);
    if typing { h.space = false; h.s = false; }

    // Houdini style: Space navigates while held, and plays on a tap.
    if nav.style == NavStyle::Houdini && !typing {
        if keyboard.just_pressed(KeyCode::Space) { drag.space_used = false; }
        if h.space && (h.lmb || h.mmb || h.rmb) { drag.space_used = true; }
        if keyboard.just_released(KeyCode::Space) && !drag.space_used { playback.playing = !playback.playing; }
    }

    let cursor = windows.get_single().ok().and_then(|w| w.cursor_position());
    let in_viewport = super::pointer_in_viewport(ctx, vp_rect.0, cursor);

    // A drag belongs to the camera only if it began in the viewport.
    if mouse_btn.get_just_pressed().next().is_some() && !(h.lmb && h.mmb && drag.active) {
        drag.active = in_viewport;
    }
    if !(h.lmb || h.mmb || h.rmb) { drag.active = false; }

    let action = if drag.active { nav.style.action(h) } else { None };
    // Movement is measured from the cursor position, not from raw device
    // motion: remote desktops and mouse sharing tools move the cursor
    // without sending usable device motion.
    let mut delta = Vec2::ZERO;
    for ev in cursor_moved.read() {
        if let Some(last) = drag.last { delta += ev.position - last; }
        drag.last = Some(ev.position);
    }
    // A jump of this size is the cursor being placed, not dragged.
    if delta.length() > 400.0 { delta = Vec2::ZERO; }
    let wheel: f32 = mouse_wheel.read().map(|ev| ev.y).sum();

    for mut t in cam_q.iter_mut() {
        match action {
            Some(NavAction::Orbit) => {
                let off   = t.translation - orbit.target;
                let yaw   = Quat::from_rotation_y(-delta.x * 0.01);
                let right = *t.right();
                let pitch = Quat::from_axis_angle(right, -delta.y * 0.01);
                t.translation = orbit.target + pitch * (yaw * off);
                t.look_at(orbit.target, Vec3::Y);
            }
            Some(NavAction::Pan) => {
                // Speed follows the distance, so the scene tracks the cursor.
                let k = (t.translation.distance(orbit.target) * 0.0015).max(0.0005) * nav.style.pan_sign();
                let d = *t.right() * -delta.x * k + *t.up() * delta.y * k;
                t.translation += d;
                orbit.target  += d;
            }
            Some(NavAction::Zoom) => zoom(&mut t, orbit.target, nav.zoom(delta) * 0.01),
            None => {}
        }
        if wheel != 0.0 && in_viewport { zoom(&mut t, orbit.target, wheel * 0.12); }
    }
}

/// Move towards the orbit target by a fraction of the distance to it, so
/// zooming slows down close up and never passes through the target.
fn zoom(t: &mut Transform, target: Vec3, amount: f32) {
    let off = t.translation - target;
    let dist = (off.length() * (-amount).exp()).clamp(0.02, 1.0e5);
    t.translation = target + off.normalize_or_zero() * dist;
}

/// Where the camera has to be to see a sphere, looking along `dir`.
/// Returns the new position. `fov` is the vertical field of view in radians.
pub fn frame_position(centre: Vec3, radius: f32, dir: Vec3, fov: f32, aspect: f32) -> Vec3 {
    // The narrower of the two view angles has to hold the sphere.
    let half_v = fov * 0.5;
    let half_h = (half_v.tan() * aspect.max(0.05)).atan();
    let half = half_v.min(half_h).max(0.01);
    let dir = if dir.length_squared() > 1e-10 { dir.normalize() } else { Vec3::new(0.5, 0.45, 0.74).normalize() };
    centre + dir * (radius.max(1e-3) / half.sin() * 1.15)
}

/// Bounding sphere of some points: centre of their box, and its half diagonal.
pub fn bounds_of(points: impl Iterator<Item = Vec3>) -> Option<(Vec3, f32)> {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut any = false;
    for p in points { if p.is_finite() { lo = lo.min(p); hi = hi.max(p); any = true; } }
    any.then(|| ((lo + hi) * 0.5, ((hi - lo) * 0.5).length()))
}

/// Frame the view.
///
/// F, G, Z or . : frame the selection. With an Edit Poly node selected that
/// is its selected vertices, edges or polygons, or its whole mesh when
/// nothing is selected. Otherwise it is everything shown.
/// A or H: frame everything shown.
///
/// The keys cover what Maya (F, A), Houdini (G, H), Max (Z), Blender (.)
/// and Modo (A) use. The cursor has to be over the viewport.
pub fn focus_camera(
    keyboard:     Res<ButtonInput<KeyCode>>,
    graph:        Res<crate::node_graph::NodeGraphState>,
    subnets:      Res<crate::ice::SubnetStore>,
    playback:     Res<crate::timeline::Playback>,
    vp_rect:      Res<ViewportRect>,
    windows:      Query<&Window>,
    mut cam_q:    Query<(&mut Transform, &Projection), With<MainCamera>>,
    mut orbit:    ResMut<CameraOrbitState>,
    mut contexts: EguiContexts,
) {
    let selection_key = keyboard.any_just_pressed([KeyCode::KeyF, KeyCode::KeyG, KeyCode::KeyZ, KeyCode::Period, KeyCode::NumpadDecimal]);
    let all_key = keyboard.any_just_pressed([KeyCode::KeyA, KeyCode::KeyH]);
    if !selection_key && !all_key { return; }
    if keyboard.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::AltLeft, KeyCode::AltRight,
                             KeyCode::SuperLeft, KeyCode::SuperRight, KeyCode::Space, KeyCode::KeyS]) { return; }
    let ctx = contexts.ctx_mut();
    if ctx.wants_keyboard_input() { return; }
    let cursor = windows.get_single().ok().and_then(|w| w.cursor_position());
    let Some(rect) = vp_rect.0 else { return };
    if !cursor.map(|c| rect.contains(bevy_egui::egui::pos2(c.x, c.y))).unwrap_or(false) { return; }

    let eval = |sid: crate::types::SubnetId, mesh: &crate::core::geo::Geo, template: Option<&crate::core::geo::Geo>| {
        subnets.get(sid).map(|sg| sg.evaluate(mesh, template)).unwrap_or_else(|| mesh.clone())
    };

    // What to frame.
    let mut target: Option<(Vec3, f32)> = None;
    let stage = crate::modelling::stage(&graph, &eval);
    if let Some(stage) = &stage {
        if selection_key {
            let picked = stage.selection.vertex_set(&stage.mesh);
            target = bounds_of(stage.mesh.verts.iter().enumerate().filter(|(v, _)| picked[*v]).map(|(_, p)| *p));
            // One vertex has no size: show its surroundings.
            if let Some((_, r)) = &mut target {
                let whole = bounds_of(stage.mesh.verts.iter().copied()).map(|b| b.1).unwrap_or(1.0);
                *r = r.max(whole * 0.08);
            }
        }
        if target.is_none() { target = bounds_of(stage.mesh.verts.iter().copied()); }
    }
    if target.is_none() {
        let mut points: Vec<Vec3> = vec![];
        match graph.evaluate_for_viewport_packed(&eval) {
            // Packed primitives by their boxes: instanced copies are not
            // made into one mesh just to be measured.
            Some(crate::types::EvalResult::Named(prims)) => {
                for (lo, hi) in prims.iter().filter_map(crate::viewport::bounds::world_bounds) { points.extend([lo, hi]); }
            }
            Some(other) => {
                let mesh = other.shared_mesh();
                points.extend(mesh.vertices.iter().map(|v| Vec3::from_array(*v)));
                points.extend(mesh.points.iter().map(|v| Vec3::from_array(*v)));
                points.extend(mesh.curve_points.iter().map(|v| Vec3::from_array(*v)));
            }
            None => {}
        }
        for clip in graph.display_clips() {
            points.extend(clip.world_pose(clip.index_at(playback.time)).iter().map(|m| m.w_axis.truncate()));
        }
        target = bounds_of(points.into_iter());
    }
    let Some((centre, radius)) = target else { return };

    let aspect = rect.width() / rect.height().max(1.0);
    for (mut t, projection) in cam_q.iter_mut() {
        let fov = match projection { Projection::Perspective(p) => p.fov, _ => std::f32::consts::FRAC_PI_4 };
        let dir = t.translation - orbit.target;
        t.translation = frame_position(centre, radius, dir, fov, aspect);
        t.look_at(centre, Vec3::Y);
    }
    orbit.target = centre;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_backs_off_far_enough_to_see_the_whole_sphere() {
        let fov = std::f32::consts::FRAC_PI_4;
        let centre = Vec3::new(1.0, 2.0, 3.0);
        for (radius, aspect) in [(1.0, 1.6), (0.01, 1.0), (50.0, 0.5)] {
            let p = frame_position(centre, radius, Vec3::new(1.0, 1.0, 1.0), fov, aspect);
            let d = p.distance(centre);
            // The sphere fits inside the narrower view angle, with some room.
            let half = (fov * 0.5).min(((fov * 0.5).tan() * aspect).atan());
            assert!((radius / d).asin() < half, "radius {radius}");
            assert!(d < radius / half.sin() * 1.3);
            // The view direction is kept.
            assert!(((p - centre).normalize() - Vec3::splat(1.0).normalize()).length() < 1e-5);
        }
        // No direction to keep: a default one is used.
        assert!(frame_position(centre, 1.0, Vec3::ZERO, fov, 1.0).is_finite());
    }

    #[test]
    fn bounds_cover_the_points() {
        let (c, r) = bounds_of([Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 4.0, 0.0)].into_iter()).unwrap();
        assert_eq!(c, Vec3::new(1.0, 2.0, 0.0));
        assert!((r - (1.0f32 + 4.0).sqrt()).abs() < 1e-6);
        assert!(bounds_of(std::iter::empty()).is_none());
        // A single point: a sphere of no size at that point.
        assert_eq!(bounds_of([Vec3::ONE].into_iter()), Some((Vec3::ONE, 0.0)));
    }
}

pub fn draw_origin_label(
    mut contexts: EguiContexts,
    cam_q:        Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    vp_rect:      Res<ViewportRect>,
) {
    let Ok((cam, ct)) = cam_q.get_single() else { return; };
    // Only while the viewport pane is showing, and inside it.
    let Some(rect) = vp_rect.0 else { return };
    if let Some(sp) = cam.world_to_viewport(ct, Vec3::ZERO).map(|p| p + Vec2::new(rect.min.x, rect.min.y)) {
        if !rect.shrink(12.0).contains(bevy_egui::egui::pos2(sp.x, sp.y)) { return; }
        let ctx = contexts.ctx_mut();
        bevy_egui::egui::Area::new("origin_label".into())
            .order(bevy_egui::egui::Order::Background)
            .fixed_pos(bevy_egui::egui::pos2(sp.x, sp.y))
            .interactable(false)
            .show(ctx, |ui| {
                bevy_egui::egui::Frame::none()
                    .fill(bevy_egui::egui::Color32::from_rgba_premultiplied(0,0,0,180))
                    .inner_margin(4.0)
                    .show(ui, |ui| {
                        ui.label(bevy_egui::egui::RichText::new("0,0,0")
                            .color(bevy_egui::egui::Color32::WHITE).small());
                    });
            });
    }
}