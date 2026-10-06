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

    let in_viewport = windows.get_single().ok()
        .and_then(|w| w.cursor_position())
        .zip(vp_rect.0)
        .map(|(c, r)| r.contains(bevy_egui::egui::pos2(c.x, c.y)))
        .unwrap_or(false)
        && !ctx.is_pointer_over_area();

    // A drag belongs to the camera only if it began in the viewport.
    if mouse_btn.get_just_pressed().next().is_some() && !(h.lmb && h.mmb && drag.active) {
        drag.active = in_viewport && !ctx.wants_pointer_input();
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

pub fn focus_camera(
    keyboard:     Res<ButtonInput<KeyCode>>,
    mut cam_q:    Query<&mut Transform, With<MainCamera>>,
    mesh_q:       Query<&Transform, (With<GeneratedMesh>, Without<MainCamera>)>,
    mut contexts: EguiContexts,
) {
    if !keyboard.just_pressed(KeyCode::KeyF) { return; }
    if contexts.ctx_mut().is_pointer_over_area() { return; }
    if let Ok(mt) = mesh_q.get_single() {
        for mut ct in cam_q.iter_mut() {
            let dir = (ct.translation - mt.translation).normalize();
            ct.translation = mt.translation + dir * 6.0;
            ct.look_at(mt.translation, Vec3::Y);
        }
    }
}

pub fn draw_origin_label(
    mut contexts: EguiContexts,
    cam_q:        Query<(&Camera, &GlobalTransform), With<MainCamera>>,
) {
    let Ok((cam, ct)) = cam_q.get_single() else { return; };
    if let Some(sp) = cam.world_to_viewport(ct, Vec3::ZERO) {
        let ctx = contexts.ctx_mut();
        bevy_egui::egui::Area::new("origin_label".into())
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