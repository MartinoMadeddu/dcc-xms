use bevy::prelude::Vec3;
use bevy_egui::egui;
use std::sync::Arc;
use crate::types::{NodeType, RetimeMode, node_type_label};
use crate::core::anim::{AnimData, FrameRate, RATE_PRESETS};
use crate::node_graph::NodeGraphState;
use crate::scene_graph::SceneGraph;
use crate::ice::{SubnetStore, GraphNavigation};
use crate::ice::ui::draw_subnet_node_properties;

mod xsi {
    use bevy_egui::egui::Color32;
    pub const PANEL_BG:    Color32 = Color32::from_rgb(118, 118, 118);
    pub const SECTION_BG:  Color32 = Color32::from_rgb(108, 108, 108);
    pub const HEADER_TEXT: Color32 = Color32::from_rgb(230, 230, 230);
    pub const LABEL:       Color32 = Color32::from_rgb(210, 210, 210);
    pub const DIM:         Color32 = Color32::from_rgb(170, 170, 170);
}

/// Clips around the selected node and the playhead, so animation nodes can
/// show what they receive and set parameters from the current frame.
pub struct AnimContext {
    /// Playhead on the timecode axis, seconds.
    pub time:   f64,
    pub output: Option<Arc<AnimData>>,
    pub input:  Option<Arc<AnimData>>,
}

pub fn draw_properties_panel(
    ui:      &mut egui::Ui,
    graph:   &mut NodeGraphState,
    scene:   &SceneGraph,
    subnets: &mut SubnetStore,
    nav:     &GraphNavigation,
    anim:    &AnimContext,
) {
    egui::Frame::none()
        .fill(xsi::PANEL_BG)
        .inner_margin(6.0)
        .show(ui, |ui| {
            if let Some(sid) = nav.current_subnet {
                if let Some(sg) = subnets.get_mut(sid) {
                    draw_subnet_node_properties(ui, sg);
                    return;
                }
            }
            draw_properties(ui, graph, scene, anim);
        });
}

pub fn draw_properties(
    ui:     &mut egui::Ui,
    graph:  &mut NodeGraphState,
    _scene: &SceneGraph,
    anim:   &AnimContext,
) {
    ui.colored_label(xsi::HEADER_TEXT,
        egui::RichText::new("Properties").strong().size(14.0));
    ui.separator();

    let sel_id = match graph.selected_node {
        Some(id) => id,
        None => { ui.colored_label(xsi::DIM, "No node selected."); return; }
    };

    let (node_name, type_str) = match graph.nodes.iter().find(|n| n.id == sel_id) {
        Some(n) => (n.name.clone(), node_type_label(&n.node_type)),
        None    => return,
    };

    // Name section
    section(ui, |ui| {
        ui.colored_label(xsi::DIM, type_str);
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.colored_label(xsi::LABEL, "Name:");
            if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == sel_id) {
                ui.text_edit_singleline(&mut node.name);
            }
        });
    });

    ui.add_space(4.0);

    // Type-specific parameters
    if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == sel_id) {
        match &mut node.node_type {
            NodeType::CreateCube { size } => {
                section_label(ui, "Geometry");
                section(ui, |ui| {
                    labeled_slider(ui, "Size", size, 0.1..=5.0);
                });
            }
            NodeType::CreateSphere { radius, segments } => {
                section_label(ui, "Geometry");
                section(ui, |ui| {
                    labeled_slider(ui, "Radius",   radius,   0.1..=3.0);
                    labeled_slider_u32(ui, "Segments", segments, 4..=64);
                });
            }
            NodeType::CreateGrid { rows, cols, size } => {
                section_label(ui, "Geometry");
                section(ui, |ui| {
                    labeled_slider_u32(ui, "Rows", rows, 1..=100);
                    labeled_slider_u32(ui, "Cols", cols, 1..=100);
                    labeled_slider(ui, "Size",     size, 0.1..=20.0);
                });
            }
            NodeType::LoadUsd { path } => {
                ui.label("USD File Path");
                ui.separator();

                ui.label("Path:");
                let mut buf = path.clone();
                let changed = ui.add(
                    egui::TextEdit::singleline(&mut buf)
                        .hint_text("/path/to/file.usda")
                        .desired_width(f32::INFINITY),
                ).changed();
                if changed {
                    *path = buf;
                }

                // Quick file-existence indicator
                if path.is_empty() {
                    ui.label(egui::RichText::new("No file set").color(egui::Color32::from_gray(140)));
                } else if std::path::Path::new(path).exists() {
                    ui.label(egui::RichText::new("✔ File found").color(egui::Color32::from_rgb(140, 200, 140)));
                } else {
                    ui.label(egui::RichText::new("✘ File not found").color(egui::Color32::from_rgb(200, 120, 120)));
                }

                ui.separator();
                ui.label(egui::RichText::new("Supported: .usda  .usdc  .usdz")
                    .color(egui::Color32::from_gray(160))
                    .small());
            }
            NodeType::Transform { translation, rotation, scale } => {
                section_label(ui, "Translation");
                section(ui, |ui| { drag_vec3(ui, translation, 0.05); });
                ui.add_space(4.0);
                section_label(ui, "Rotation (deg)");
                section(ui, |ui| {
                    for (lbl, v) in [("X", &mut rotation.x), ("Y", &mut rotation.y), ("Z", &mut rotation.z)] {
                        ui.horizontal(|ui| {
                            ui.colored_label(xsi::LABEL, format!("{lbl}:"));
                            let mut deg = v.to_degrees();
                            if ui.add(egui::DragValue::new(&mut deg).speed(1.0)).changed() {
                                *v = deg.to_radians();
                            }
                        });
                    }
                });
                ui.add_space(4.0);
                section_label(ui, "Scale");
                section(ui, |ui| { drag_vec3(ui, scale, 0.01); });
                ui.add_space(4.0);
                if ui.button("Reset All").clicked() {
                    *translation = Vec3::ZERO;
                    *rotation    = Vec3::ZERO;
                    *scale       = Vec3::ONE;
                }
            }
            NodeType::ScatterPoints { count, seed } => {
                section_label(ui, "Distribution");
                section(ui, |ui| {
                    labeled_slider_u32(ui, "Count", count, 1..=10_000);
                    labeled_slider_u32(ui, "Seed",  seed,  0..=9_999);
                });
            }
            NodeType::CopyToPoints => {
                section(ui, |ui| {
                    ui.colored_label(xsi::LABEL, "Input 0 → Template mesh");
                    ui.colored_label(xsi::LABEL, "Input 1 → Point cloud");
                });
            }
            NodeType::Subnet { .. } => {
                section(ui, |ui| {
                    ui.colored_label(xsi::DIM, "Dive in with double-click.");
                });
            }
            NodeType::Merge | NodeType::Output => {
                section(ui, |ui| {
                    ui.colored_label(xsi::DIM, "No editable parameters.");
                });
            }

            // ── Animation ────────────────────────────────────────────────────
            NodeType::LoadFbx { path, take } => {
                section_label(ui, "FBX File");
                section(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(path)
                        .hint_text("/path/to/take.fbx")
                        .desired_width(f32::INFINITY));
                    match crate::fbx_loader::load_fbx_cached(path, *take) {
                        Ok(loaded) => {
                            ui.label(egui::RichText::new("✔ Loaded").color(egui::Color32::from_rgb(140, 200, 140)));
                            if loaded.takes.len() > 1 {
                                let cur = (*take as usize).min(loaded.takes.len() - 1);
                                egui::ComboBox::from_label("Take")
                                    .selected_text(loaded.takes[cur].clone())
                                    .show_ui(ui, |ui| {
                                        for (i, name) in loaded.takes.iter().enumerate() {
                                            ui.selectable_value(take, i as u32, name);
                                        }
                                    });
                            }
                        }
                        Err(e) => {
                            ui.label(egui::RichText::new(format!("✘ {e}")).color(egui::Color32::from_rgb(200, 120, 120)));
                        }
                    }
                });
                ui.colored_label(xsi::DIM, egui::RichText::new(
                    "Skeleton and one take, baked per frame. Converted to Y-up, metres. Meshes are not read.").small());
            }
            NodeType::TestClip { seconds, fps_num, fps_den } => {
                section_label(ui, "Clip");
                section(ui, |ui| {
                    labeled_slider(ui, "Seconds", seconds, 0.5..=60.0);
                    rate_combo(ui, "test_clip_rate", fps_num, fps_den);
                });
            }
            NodeType::RenameJoints { find, replace, strip_namespace, prefix } => {
                section_label(ui, "Rename");
                section(ui, |ui| {
                    ui.checkbox(strip_namespace, "Strip namespace (text before last ':')");
                    ui.horizontal(|ui| { ui.colored_label(xsi::LABEL, "Find:");    ui.text_edit_singleline(find); });
                    ui.horizontal(|ui| { ui.colored_label(xsi::LABEL, "Replace:"); ui.text_edit_singleline(replace); });
                    ui.horizontal(|ui| { ui.colored_label(xsi::LABEL, "Prefix:");  ui.text_edit_singleline(prefix); });
                });
                if let (Some(i), Some(o)) = (&anim.input, &anim.output) {
                    ui.add_space(4.0);
                    section_label(ui, "Result");
                    section(ui, |ui| {
                        egui::ScrollArea::vertical().id_source("rename_preview").max_height(220.0).show(ui, |ui| {
                            for (a, b) in i.joints.iter().zip(o.joints.iter()) {
                                let col = if a.name == b.name { xsi::DIM } else { xsi::HEADER_TEXT };
                                ui.colored_label(col, format!("{}  >  {}", a.name, b.name));
                            }
                        });
                    });
                }
            }
            NodeType::TrimClip { head, tail } => {
                section_label(ui, "Trim (frames removed)");
                section(ui, |ui| {
                    let max = anim.input.as_ref().map(|i| i.frames.saturating_sub(1) as u32).unwrap_or(100_000);
                    ui.horizontal(|ui| {
                        ui.colored_label(xsi::LABEL, "Head:");
                        ui.add(egui::DragValue::new(head).range(0..=max));
                        ui.colored_label(xsi::LABEL, "Tail:");
                        ui.add(egui::DragValue::new(tail).range(0..=max));
                    });
                    if let Some(i) = &anim.input {
                        let playhead = i.frame_at(anim.time).clamp(i.start_frame, i.end_frame());
                        ui.horizontal(|ui| {
                            if ui.button("In = playhead").clicked() {
                                *head = (playhead - i.start_frame) as u32;
                                *tail = (*tail).min(max - *head);
                            }
                            if ui.button("Out = playhead").clicked() {
                                *tail = (i.end_frame() - playhead) as u32;
                                *head = (*head).min(max - *tail);
                            }
                            if ui.button("Reset").clicked() { *head = 0; *tail = 0; }
                        });
                    }
                });
                clip_summary(ui, anim);
            }
            NodeType::Retime { fps_num, fps_den, mode } => {
                section_label(ui, "Retime");
                section(ui, |ui| {
                    rate_combo(ui, "retime_rate", fps_num, fps_den);
                    ui.radio_value(mode, RetimeMode::Resample,    "Resample (keep duration)");
                    ui.radio_value(mode, RetimeMode::Reinterpret, "Reinterpret (keep frames)");
                });
                clip_summary(ui, anim);
            }
            NodeType::SetTimecode { hours, minutes, seconds, frames, drop_frame } => {
                section_label(ui, "Start timecode");
                section(ui, |ui| {
                    let tb = anim.input.as_ref().map(|i| i.rate.timebase() as u32).unwrap_or(120);
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(hours).range(0..=23));
                        ui.label(":");
                        ui.add(egui::DragValue::new(minutes).range(0..=59));
                        ui.label(":");
                        ui.add(egui::DragValue::new(seconds).range(0..=59));
                        ui.label(":");
                        ui.add(egui::DragValue::new(frames).range(0..=tb.saturating_sub(1)));
                    });
                    let can_drop = anim.input.as_ref().map(|i| i.rate.supports_drop_frame()).unwrap_or(true);
                    ui.add_enabled(can_drop, egui::Checkbox::new(drop_frame, "Drop frame (29.97 / 59.94 only)"));
                });
                clip_summary(ui, anim);
            }
        }
    }

    ui.add_space(6.0);
    ui.colored_label(xsi::DIM, format!("「{}」", node_name));
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn rate_combo(ui: &mut egui::Ui, id: &str, num: &mut u32, den: &mut u32) {
    let cur = FrameRate::new(*num, *den);
    ui.horizontal(|ui| {
        ui.colored_label(xsi::LABEL, "Rate:");
        egui::ComboBox::from_id_source(id)
            .selected_text(format!("{} fps", cur.label()))
            .show_ui(ui, |ui| {
                for (name, r) in RATE_PRESETS {
                    if ui.selectable_label(*r == cur, format!("{name} fps")).clicked() {
                        *num = r.num;
                        *den = r.den;
                    }
                }
            });
    });
}

/// In / out of the selected animation node.
fn clip_summary(ui: &mut egui::Ui, anim: &AnimContext) {
    let line = |c: &AnimData| format!(
        "{} - {}   {} frames @ {} fps{}",
        c.timecode(c.start_frame), c.timecode(c.end_frame()),
        c.frames, c.rate.label(), if c.drop_frame { " DF" } else { "" });
    ui.add_space(4.0);
    section(ui, |ui| {
        match &anim.input {
            Some(i) => { ui.colored_label(xsi::DIM, format!("In:   {}", line(i))); }
            None    => { ui.colored_label(xsi::DIM, "In:   no clip connected"); }
        }
        if let Some(o) = &anim.output {
            ui.colored_label(xsi::LABEL, format!("Out:  {}", line(o)));
        }
    });
}

fn section(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(xsi::SECTION_BG)
        .inner_margin(egui::vec2(8.0, 6.0))
        .rounding(4.0)
        .show(ui, add);
}

fn section_label(ui: &mut egui::Ui, label: &str) {
    ui.colored_label(xsi::DIM, egui::RichText::new(label).size(10.0).strong());
    ui.add_space(2.0);
}

fn labeled_slider(ui: &mut egui::Ui, label: &str, val: &mut f32, range: std::ops::RangeInclusive<f32>) {
    ui.horizontal(|ui| {
        ui.colored_label(xsi::LABEL, format!("{label}:"));
        ui.add(egui::Slider::new(val, range));
    });
}

fn labeled_slider_u32(ui: &mut egui::Ui, label: &str, val: &mut u32, range: std::ops::RangeInclusive<u32>) {
    ui.horizontal(|ui| {
        ui.colored_label(xsi::LABEL, format!("{label}:"));
        ui.add(egui::Slider::new(val, range));
    });
}

fn drag_vec3(ui: &mut egui::Ui, v: &mut Vec3, speed: f64) {
    ui.horizontal(|ui| {
        ui.colored_label(xsi::LABEL, "X:"); ui.add(egui::DragValue::new(&mut v.x).speed(speed));
        ui.colored_label(xsi::LABEL, "Y:"); ui.add(egui::DragValue::new(&mut v.y).speed(speed));
        ui.colored_label(xsi::LABEL, "Z:"); ui.add(egui::DragValue::new(&mut v.z).speed(speed));
    });
}