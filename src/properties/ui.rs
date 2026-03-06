use bevy::prelude::Vec3;
use bevy_egui::egui;
use crate::types::{NodeType, node_type_label};
use crate::node_graph::NodeGraphState;
use crate::scene_graph::SceneGraph;
use crate::subnet::{SubnetStore, GraphNavigation};
use crate::subnet::ui::draw_subnet_node_properties;

mod xsi {
    use bevy_egui::egui::Color32;
    pub const PANEL_BG:    Color32 = Color32::from_rgb(118, 118, 118);
    pub const SECTION_BG:  Color32 = Color32::from_rgb(108, 108, 108);
    pub const HEADER_TEXT: Color32 = Color32::from_rgb(230, 230, 230);
    pub const LABEL:       Color32 = Color32::from_rgb(210, 210, 210);
    pub const DIM:         Color32 = Color32::from_rgb(170, 170, 170);
}

pub fn draw_properties_panel(
    ui:      &mut egui::Ui,
    graph:   &mut NodeGraphState,
    scene:   &SceneGraph,
    subnets: &mut SubnetStore,
    nav:     &GraphNavigation,
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
            draw_properties(ui, graph, scene);
        });
}

pub fn draw_properties(
    ui:     &mut egui::Ui,
    graph:  &mut NodeGraphState,
    _scene: &SceneGraph,
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
        }
    }

    ui.add_space(6.0);
    ui.colored_label(xsi::DIM, format!("「{}」", node_name));
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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