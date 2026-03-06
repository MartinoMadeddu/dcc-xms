use bevy::prelude::*;
use bevy_egui::EguiContexts;
use crate::types::{MainCamera, GeneratedMesh};

#[derive(Resource, Default)]
pub struct CameraOrbitState {
    pub target: Vec3,
}

pub fn camera_controller(
    mouse_btn:        Res<ButtonInput<MouseButton>>,
    mut mouse_motion: EventReader<bevy::input::mouse::MouseMotion>,
    mut mouse_wheel:  EventReader<bevy::input::mouse::MouseWheel>,
    keyboard:         Res<ButtonInput<KeyCode>>,
    mut cam_q:        Query<&mut Transform, With<MainCamera>>,
    mut orbit:        ResMut<CameraOrbitState>,
    mut contexts:     EguiContexts,
) {
    if contexts.ctx_mut().wants_pointer_input() {
        mouse_motion.clear(); mouse_wheel.clear(); return;
    }
    let alt = keyboard.pressed(KeyCode::AltLeft)   || keyboard.pressed(KeyCode::AltRight)
           || keyboard.pressed(KeyCode::SuperLeft)  || keyboard.pressed(KeyCode::SuperRight);
    if !alt { mouse_motion.clear(); mouse_wheel.clear(); return; }

    for mut t in cam_q.iter_mut() {
        if mouse_btn.pressed(MouseButton::Left) {
            for ev in mouse_motion.read() {
                let off   = t.translation - orbit.target;
                let yaw   = Quat::from_rotation_y(-ev.delta.x * 0.01);
                let right = *t.right();
                let pitch = Quat::from_axis_angle(right, -ev.delta.y * 0.01);
                t.translation = orbit.target + pitch * (yaw * off);
                t.look_at(orbit.target, Vec3::Y);
            }
        }
        if mouse_btn.pressed(MouseButton::Middle) {
            for ev in mouse_motion.read() {
                let d = *t.right() * -ev.delta.x * 0.01
                      + *t.up()    *  ev.delta.y * 0.01;
                t.translation += d;
                orbit.target  += d;
            }
        }
        if mouse_btn.pressed(MouseButton::Right) {
            for ev in mouse_motion.read() {
                let fwd = *t.forward();
                t.translation += fwd * ev.delta.y * 0.02;
            }
        }
        for ev in mouse_wheel.read() {
            let dir = (orbit.target - t.translation).normalize();
            t.translation += dir * ev.y * 0.5;
        }
    }
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