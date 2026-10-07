use bevy::prelude::Vec3;
use bevy_egui::egui;
use std::sync::Arc;
use crate::types::{NodeType, RetimeMode, node_type_label};
use crate::core::anim::{AnimData, FrameRate, PoseEdit, RATE_PRESETS};
use crate::types::{NodeId, SplitPick};
use crate::file_browser::{BrowseMode, BrowseTarget, FileBrowser};
use crate::batch::{self, BatchState};
use crate::core::manip::Tool;
use crate::core::poly::{collapse_all, collapse_op, eval_cached, push_op, restore_run, ExtrudeMode, PolyMesh, PolyOp, PolyOpKind, PolySelection, SelSource, SubLevel};
use bevy::math::{EulerRot, Quat};
use crate::node_graph::NodeGraphState;
use crate::scene_graph::SceneGraph;
use crate::ice::{SubnetStore, GraphNavigation};
use crate::ice::ui::draw_subnet_node_properties;

mod xsi {
    // Light-theme colours. `theme::c` returns the dark counterpart in dark mode.
    #![allow(non_snake_case)]
    use bevy_egui::egui::Color32;
    pub fn PANEL_BG() -> Color32 { crate::theme::c(118, 118, 118) }
    pub fn SECTION_BG() -> Color32 { crate::theme::c(108, 108, 108) }
    pub fn HEADER_TEXT() -> Color32 { crate::theme::c(230, 230, 230) }
    pub fn LABEL() -> Color32 { crate::theme::c(210, 210, 210) }
    pub fn DIM() -> Color32 { crate::theme::c(170, 170, 170) }
}

/// Clips around the selected node and the playhead, so animation nodes can
/// show what they receive and set parameters from the current frame.
pub struct AnimContext {
    /// Playhead on the timecode axis, seconds.
    pub time:   f64,
    pub output: Option<Arc<AnimData>>,
    pub input:  Option<Arc<AnimData>>,
}

/// Things a node's parameters can ask the application to do.
pub enum PanelAction {
    /// Run these Write FBX nodes, for the current file or the whole folder.
    Write { targets: Vec<NodeId>, all_files: bool },
}

/// Services the panel can reach beyond the graph.
pub struct PanelIo<'a> {
    pub browser: &'a mut FileBrowser,
    pub batch:   &'a BatchState,
    pub action:  Option<PanelAction>,
    /// Mesh entering the selected node, when it is an Edit Poly node.
    pub poly_input: Option<crate::core::poly::PolyMesh>,
    /// Viewport tool of the Edit Poly node.
    pub tool: &'a mut Tool,
}

pub fn draw_properties_panel(
    ui:      &mut egui::Ui,
    graph:   &mut NodeGraphState,
    scene:   &SceneGraph,
    subnets: &mut SubnetStore,
    nav:     &GraphNavigation,
    anim:    &AnimContext,
    io:      &mut PanelIo,
) {
    egui::Frame::none()
        .fill(xsi::PANEL_BG())
        .inner_margin(6.0)
        .show(ui, |ui| {
            if let Some(sid) = nav.current_subnet {
                if let Some(sg) = subnets.get_mut(sid) {
                    draw_subnet_node_properties(ui, sg);
                    return;
                }
            }
            draw_properties(ui, graph, scene, anim, io);
        });
}

pub fn draw_properties(
    ui:     &mut egui::Ui,
    graph:  &mut NodeGraphState,
    _scene: &SceneGraph,
    anim:   &AnimContext,
    io:     &mut PanelIo,
) {
    ui.colored_label(xsi::HEADER_TEXT(),
        egui::RichText::new("Properties").strong().size(14.0));
    ui.separator();

    let sel_id = match graph.selected_node {
        Some(id) => id,
        None => { ui.colored_label(xsi::DIM(), "No node selected."); return; }
    };

    let (node_name, type_str) = match graph.nodes.iter().find(|n| n.id == sel_id) {
        Some(n) => (n.name.clone(), node_type_label(&n.node_type)),
        None    => return,
    };

    // Name section
    section(ui, |ui| {
        ui.colored_label(xsi::DIM(), type_str);
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.colored_label(xsi::LABEL(), "Name:");
            if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == sel_id) {
                ui.text_edit_singleline(&mut node.name);
            }
        });
    });

    ui.add_space(4.0);

    // Facts about the graph the parameter widgets need, gathered before the
    // node is borrowed for editing.
    let all_writers  = batch::write_nodes(graph);
    let batch_files  = batch::file_count(graph, &batch::upstream_folder_loaders(graph, &[sel_id]));
    let mut resync   = false;

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
                path_row(ui, path, "/path/to/file.usda", io, sel_id, BrowseMode::File, "Open USD", &["usda", "usdc", "usdz", "usd"]);

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
                            ui.colored_label(xsi::LABEL(), format!("{lbl}:"));
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
                    ui.colored_label(xsi::LABEL(), "Input 0 → Template mesh");
                    ui.colored_label(xsi::LABEL(), "Input 1 → Point cloud");
                });
            }
            NodeType::Subnet { .. } => {
                section(ui, |ui| {
                    ui.colored_label(xsi::DIM(), "Dive in with double-click.");
                });
            }
            NodeType::Merge | NodeType::Output => {
                section(ui, |ui| {
                    ui.colored_label(xsi::DIM(), "No editable parameters.");
                });
            }

            // ── Animation ────────────────────────────────────────────────────
            NodeType::LoadFbx { path, take } => {
                section_label(ui, "FBX File");
                section(ui, |ui| {
                    path_row(ui, path, "/path/to/take.fbx", io, sel_id, BrowseMode::File, "Open FBX", &["fbx"]);
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
                ui.colored_label(xsi::DIM(), egui::RichText::new(
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
                    ui.horizontal(|ui| { ui.colored_label(xsi::LABEL(), "Find:");    ui.text_edit_singleline(find); });
                    ui.horizontal(|ui| { ui.colored_label(xsi::LABEL(), "Replace:"); ui.text_edit_singleline(replace); });
                    ui.horizontal(|ui| { ui.colored_label(xsi::LABEL(), "Prefix:");  ui.text_edit_singleline(prefix); });
                });
                if let (Some(i), Some(o)) = (&anim.input, &anim.output) {
                    ui.add_space(4.0);
                    section_label(ui, "Result");
                    section(ui, |ui| {
                        egui::ScrollArea::vertical().id_source("rename_preview").max_height(220.0).show(ui, |ui| {
                            for (a, b) in i.joints.iter().zip(o.joints.iter()) {
                                let col = if a.name == b.name { xsi::DIM() } else { xsi::HEADER_TEXT() };
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
                        ui.colored_label(xsi::LABEL(), "Head:");
                        ui.add(egui::DragValue::new(head).range(0..=max));
                        ui.colored_label(xsi::LABEL(), "Tail:");
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

            // ── Batch / export ───────────────────────────────────────────────
            NodeType::LoadFbxDir { dir, index, take } => {
                section_label(ui, "FBX Folder");
                let files = crate::fbx_loader::list_fbx(dir);
                section(ui, |ui| {
                    path_row(ui, dir, "/path/to/folder", io, sel_id, BrowseMode::Folder, "Choose FBX folder", &["fbx"]);
                    if files.is_empty() {
                        let msg = if dir.is_empty() { "No folder set" } else { "✘ No .fbx files in this folder" };
                        ui.label(egui::RichText::new(msg).color(egui::Color32::from_rgb(200, 120, 120)));
                        return;
                    }
                    let last = files.len() as u32 - 1;
                    *index = (*index).min(last);
                    let name = |i: usize| files[i].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();

                    ui.horizontal(|ui| {
                        ui.colored_label(xsi::LABEL(), "Index:");
                        if ui.add_enabled(*index > 0, egui::Button::new("◀")).clicked() { *index -= 1; }
                        ui.add(egui::DragValue::new(index).range(0..=last).speed(0.1));
                        if ui.add_enabled(*index < last, egui::Button::new("▶")).clicked() { *index += 1; }
                        ui.colored_label(xsi::DIM(), format!("of {}", files.len()));
                    });
                    egui::ComboBox::from_id_source("fbx_dir_file")
                        .width(ui.available_width() - 8.0)
                        .selected_text(name(*index as usize))
                        .show_ui(ui, |ui| {
                            for i in 0..files.len() {
                                ui.selectable_value(index, i as u32, format!("{i}  {}", name(i)));
                            }
                        });

                    match crate::fbx_loader::load_fbx_cached(&files[*index as usize].to_string_lossy(), *take) {
                        Ok(loaded) => {
                            ui.label(egui::RichText::new("✔ Loaded").color(egui::Color32::from_rgb(140, 200, 140)));
                            if loaded.takes.len() > 1 {
                                let cur = (*take as usize).min(loaded.takes.len() - 1);
                                egui::ComboBox::from_label("Take")
                                    .selected_text(loaded.takes[cur].clone())
                                    .show_ui(ui, |ui| {
                                        for (i, n) in loaded.takes.iter().enumerate() {
                                            ui.selectable_value(take, i as u32, n);
                                        }
                                    });
                            }
                        }
                        Err(e) => {
                            ui.label(egui::RichText::new(format!("✘ {e}")).color(egui::Color32::from_rgb(200, 120, 120)));
                        }
                    }
                });
                clip_summary(ui, anim);
            }

            NodeType::SplitSkeleton { picks } => {
                section_label(ui, "Outputs (one per character)");
                let input = anim.input.clone();
                let chars: Vec<(usize, String)> = input.as_ref()
                    .map(|c| c.character_roots().into_iter().map(|r| (r, c.character_name(r))).collect())
                    .unwrap_or_default();
                section(ui, |ui| {
                    let mut remove = None;
                    for (n, pick) in picks.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.colored_label(xsi::LABEL(), format!("Char {}:", n + 1));
                            let text = match &*pick {
                                SplitPick::Character(i) => match chars.get(*i as usize) {
                                    Some((_, name)) => format!("#{} {name}", i + 1),
                                    None            => format!("#{} (not in this file)", i + 1),
                                },
                                SplitPick::Joint(name) => format!("joint {name}"),
                            };
                            egui::ComboBox::from_id_source(("split_pick", n))
                                .width(150.0)
                                .selected_text(text)
                                .show_ui(ui, |ui| {
                                    ui.weak("Characters, by position");
                                    for (i, (_, name)) in chars.iter().enumerate() {
                                        ui.selectable_value(pick, SplitPick::Character(i as u32), format!("#{} {name}", i + 1));
                                    }
                                    if let Some(c) = &input {
                                        ui.separator();
                                        ui.weak("Any joint, by name");
                                        for j in &c.joints {
                                            ui.selectable_value(pick, SplitPick::Joint(j.name.clone()), &j.name);
                                        }
                                    }
                                });
                            if ui.small_button("x").on_hover_text("Remove this output").clicked() { remove = Some(n); }
                        });
                    }
                    if let Some(n) = remove { picks.remove(n); resync = true; }
                    ui.horizontal(|ui| {
                        if ui.button("+ Add output").clicked() {
                            picks.push(SplitPick::Character(picks.len() as u32));
                            resync = true;
                        }
                        if ui.add_enabled(!chars.is_empty(), egui::Button::new("Auto-detect"))
                            .on_hover_text("One output per character found in the incoming clip")
                            .clicked()
                        {
                            *picks = (0..chars.len() as u32).map(SplitPick::Character).collect();
                            resync = true;
                        }
                    });
                });
                ui.add_space(4.0);
                section(ui, |ui| {
                    match &input {
                        None => { ui.colored_label(xsi::DIM(), "No clip connected."); }
                        Some(_) if chars.is_empty() => { ui.colored_label(xsi::DIM(), "No characters found in the incoming clip."); }
                        Some(c) => {
                            ui.colored_label(xsi::DIM(), format!("{} characters in the incoming clip:", chars.len()));
                            for (i, (root, name)) in chars.iter().enumerate() {
                                ui.colored_label(xsi::LABEL(), format!(
                                    "#{} {name}  ({} joints, root \"{}\")",
                                    i + 1, c.subtree(*root).len(), c.joints[*root].name));
                            }
                        }
                    }
                });
            }

            NodeType::AutoTPose { set_hip_height, hip_height } => {
                section_label(ui, "Neutral pose");
                section(ui, |ui| {
                    ui.colored_label(xsi::DIM(), "Rotations zeroed, root at the origin, hips centred. One frame, no animation.");
                    let rest = anim.input.as_ref().and_then(|c| c.hip_joint().map(|h| (c.joints[h].name.clone(), c.joints[h].rest.translation.y * 100.0)));
                    if let Some((name, h)) = &rest {
                        ui.colored_label(xsi::LABEL(), format!("Hips: {name}, {h:.2} cm in the file"));
                    }
                    ui.checkbox(set_hip_height, "Set hip height");
                    ui.add_enabled_ui(*set_hip_height, |ui| {
                        ui.horizontal(|ui| {
                            ui.colored_label(xsi::LABEL(), "Height:");
                            ui.add(egui::DragValue::new(hip_height).speed(0.1).range(0.0..=300.0).suffix(" cm"));
                            if let Some((_, h)) = &rest {
                                if ui.small_button("From file").clicked() { *hip_height = *h; }
                            }
                        });
                    });
                });
            }

            NodeType::FixPose { edits } => {
                section_label(ui, "Manual corrections");
                let joints: Vec<String> = anim.input.as_ref()
                    .map(|c| c.joints.iter().map(|j| j.name.clone()).collect())
                    .unwrap_or_default();
                let mut remove = None;
                for (n, e) in edits.iter_mut().enumerate() {
                    section(ui, |ui| {
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_source(("fix_joint", n))
                                .width(170.0)
                                .selected_text(if e.joint.is_empty() { "pick a joint".to_string() } else { e.joint.clone() })
                                .show_ui(ui, |ui| {
                                    for j in &joints { ui.selectable_value(&mut e.joint, j.clone(), j); }
                                });
                            if ui.small_button("x").on_hover_text("Remove").clicked() { remove = Some(n); }
                        });
                        ui.horizontal(|ui| {
                            ui.colored_label(xsi::LABEL(), "Rot");
                            for v in e.rotation.iter_mut() {
                                ui.add(egui::DragValue::new(v).speed(0.5).suffix("°"));
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.colored_label(xsi::LABEL(), "Pos");
                            for v in e.translation.iter_mut() {
                                ui.add(egui::DragValue::new(v).speed(0.1).suffix(" cm"));
                            }
                        });
                    });
                    ui.add_space(2.0);
                }
                if let Some(n) = remove { edits.remove(n); }
                if ui.button("+ Add joint").clicked() {
                    edits.push(PoseEdit { joint: String::new(), rotation: [0.0; 3], translation: [0.0; 3] });
                }
                ui.colored_label(xsi::DIM(), egui::RichText::new(
                    "Rotation X, Y, Z in degrees, added in the parent's space. Applied to every frame.").small());
            }

            NodeType::ProxySkin { thickness } => {
                section_label(ui, "Proxy mesh");
                section(ui, |ui| {
                    ui.colored_label(xsi::DIM(), "Sphere per bone, cylinder per link, each bound to one bone. Built in the pose of the first frame.");
                    labeled_slider(ui, "Thickness", thickness, 0.25..=4.0);
                    if let Some(skin) = anim.output.as_ref().and_then(|c| c.skin.as_ref()) {
                        ui.colored_label(xsi::LABEL(), format!("{} vertices, {} faces", skin.positions.len(), skin.faces.len()));
                    }
                });
            }

            NodeType::WriteFbx { path } => {
                section_label(ui, "Output file");
                section(ui, |ui| {
                    path_row(ui, path, crate::types::DEFAULT_WRITE_PATH, io, sel_id, BrowseMode::Folder, "Choose output folder", &["fbx"]);
                    ui.colored_label(xsi::DIM(), egui::RichText::new(
                        "{dir} source folder   {file} source file\n{char} character   {take} take name").small());
                    match &anim.output {
                        Some(c) => {
                            ui.colored_label(xsi::LABEL(), format!("Writes: {}", crate::fbx_writer::resolve_path(path, c).display()));
                            ui.colored_label(xsi::DIM(), format!(
                                "{} joints, {}{}",
                                c.joints.len(),
                                if c.frames > 1 { format!("{} frames", c.frames) } else { "pose only".into() },
                                match &c.skin { Some(s) => format!(", mesh {} verts", s.positions.len()), None => String::new() }));
                        }
                        None => { ui.colored_label(xsi::DIM(), "No clip connected."); }
                    }
                });
                ui.add_space(4.0);
                let running = io.batch.0.lock().unwrap().running;
                section(ui, |ui| {
                    ui.add_enabled_ui(!running, |ui| {
                        ui.colored_label(xsi::DIM(), "This node");
                        ui.horizontal(|ui| {
                            if ui.button("Write this file").clicked() {
                                io.action = Some(PanelAction::Write { targets: vec![sel_id], all_files: false });
                            }
                            if ui.add_enabled(batch_files > 0, egui::Button::new(format!("Write whole folder ({batch_files})"))).clicked() {
                                io.action = Some(PanelAction::Write { targets: vec![sel_id], all_files: true });
                            }
                        });
                        ui.colored_label(xsi::DIM(), format!("All {} Write nodes", all_writers.len()));
                        ui.horizontal(|ui| {
                            if ui.button("Write this file").clicked() {
                                io.action = Some(PanelAction::Write { targets: all_writers.clone(), all_files: false });
                            }
                            if ui.add_enabled(batch_files > 0, egui::Button::new(format!("Write whole folder ({batch_files})"))).clicked() {
                                io.action = Some(PanelAction::Write { targets: all_writers.clone(), all_files: true });
                            }
                        });
                    });
                });
                batch_log(ui, io.batch);
            }

            // ── Mocap tools ──────────────────────────────────────────────────
            NodeType::MirrorClip => {
                section_label(ui, "Mirror");
                section(ui, |ui| {
                    ui.colored_label(xsi::DIM(), egui::RichText::new(
                        "Left and right joints swap roles and the motion is reflected. Joints pair up by name: Left / Right, L_ / R_, _L / _R.").small());
                });
                clip_summary(ui, anim);
            }
            NodeType::SmoothClip { radius, amount, translations } => {
                section_label(ui, "Smooth");
                section(ui, |ui| {
                    labeled_slider_u32(ui, "Radius (frames)", radius, 0..=30);
                    labeled_slider(ui, "Amount", amount, 0.0..=1.0);
                    ui.checkbox(translations, "Also smooth positions");
                });
                clip_summary(ui, anim);
            }
            NodeType::InPlace { keep_height, to_root } => {
                section_label(ui, "In place");
                section(ui, |ui| {
                    ui.checkbox(keep_height, "Keep the up and down motion");
                    ui.checkbox(to_root, "Put the travel on the root joint")
                        .on_hover_text("Root motion: the joint above the hips carries the travel, the hips stay under it");
                });
                clip_summary(ui, anim);
            }
            NodeType::TransformClip { translate, rotate, scale } => {
                section_label(ui, "Transform clip");
                section(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(xsi::LABEL(), "Move:");
                        for a in translate.iter_mut() { ui.add(egui::DragValue::new(a).speed(0.01).max_decimals(3)); }
                    });
                    ui.horizontal(|ui| {
                        ui.colored_label(xsi::LABEL(), "Turn:");
                        for a in rotate.iter_mut() { ui.add(egui::DragValue::new(a).speed(0.5).max_decimals(2).suffix("°")); }
                    });
                    ui.horizontal(|ui| {
                        ui.colored_label(xsi::LABEL(), "Scale:");
                        ui.add(egui::DragValue::new(scale).speed(0.01).range(0.001..=1000.0).max_decimals(3));
                    });
                });
                clip_summary(ui, anim);
            }
            NodeType::BlendClips { blend, align } => {
                section_label(ui, "Blend clips");
                section(ui, |ui| {
                    labeled_slider_u32(ui, "Cross-fade (frames)", blend, 0..=240);
                    ui.checkbox(align, "Start the next clip where this one ends");
                    ui.colored_label(xsi::DIM(), egui::RichText::new(
                        "First input, then the second. Joints are matched by name.").small());
                });
                clip_summary(ui, anim);
            }
            NodeType::LoopClip { blend } => {
                section_label(ui, "Loop");
                section(ui, |ui| {
                    labeled_slider_u32(ui, "Ease over (frames)", blend, 0..=240);
                    ui.colored_label(xsi::DIM(), egui::RichText::new(
                        "The end of the clip is eased into the pose of its first frame. Travel is kept.").small());
                });
                clip_summary(ui, anim);
            }
            NodeType::Retarget => {
                section_label(ui, "Retarget");
                section(ui, |ui| {
                    ui.colored_label(xsi::DIM(), egui::RichText::new(
                        "Motion from the first input on the skeleton of the second. Joints are matched by name, ignoring namespaces and the prefix each skeleton shares. The skeletons may rest in different poses: bones are lined up at rest first.").small());
                    if let (Some(i), Some(o)) = (&anim.input, &anim.output) {
                        ui.colored_label(xsi::LABEL(), format!("Source: {} joints. Result: {} joints.", i.joints.len(), o.joints.len()));
                    } else if anim.output.is_none() {
                        ui.colored_label(xsi::LABEL(), "Connect a clip to both inputs.");
                    }
                });
                clip_summary(ui, anim);
            }
            NodeType::TimeWarp { speed, reverse } => {
                section_label(ui, "Time warp");
                section(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(xsi::LABEL(), "Speed:");
                        ui.add(egui::DragValue::new(speed).speed(0.01).range(0.05..=20.0).max_decimals(3).suffix(" x"));
                    });
                    ui.checkbox(reverse, "Play backwards");
                });
                clip_summary(ui, anim);
            }
            NodeType::PruneJoints { words } => {
                section_label(ui, "Prune joints");
                section(ui, |ui| {
                    ui.horizontal(|ui| { ui.colored_label(xsi::LABEL(), "Names containing:"); ui.text_edit_singleline(words); });
                    ui.colored_label(xsi::DIM(), egui::RichText::new(
                        "Comma-separated words, for example: finger, thumb, toe. Each matching joint goes, with everything below it.").small());
                    if let (Some(i), Some(o)) = (&anim.input, &anim.output) {
                        ui.colored_label(xsi::LABEL(), format!("{} joints in, {} out", i.joints.len(), o.joints.len()));
                    }
                });
            }
            NodeType::FloorClip { height } => {
                section_label(ui, "Floor");
                section(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(xsi::LABEL(), "Height (m):");
                        ui.add(egui::DragValue::new(height).speed(0.005).max_decimals(3));
                    });
                    if let Some(i) = &anim.input {
                        ui.colored_label(xsi::DIM(), format!("Lowest point of the incoming clip: {:.3} m", i.lowest_point()));
                    }
                });
                clip_summary(ui, anim);
            }

            // ── UV ───────────────────────────────────────────────────────────
            NodeType::UvUnwrap { method, angle, margin, axis } => {
                use crate::core::uv::UvMethod;
                section_label(ui, "UV unwrap");
                section(ui, |ui| {
                    ui.radio_value(method, UvMethod::Conformal, "Conformal (LSCM)")
                        .on_hover_text("Charts by normal angle, each flattened with Least Squares Conformal Maps, packed at equal density");
                    ui.radio_value(method, UvMethod::Box, "Box");
                    ui.radio_value(method, UvMethod::Planar, "Planar");
                    match method {
                        UvMethod::Conformal => labeled_slider(ui, "Chart angle", angle, 5.0..=89.0),
                        UvMethod::Planar => { ui.horizontal(|ui| {
                            ui.colored_label(xsi::LABEL(), "Along:");
                            ui.selectable_value(axis, 0, "X");
                            ui.selectable_value(axis, 1, "Y");
                            ui.selectable_value(axis, 2, "Z");
                        }); }
                        UvMethod::Box => {}
                    }
                    labeled_slider(ui, "Margin", margin, 0.0..=0.1);
                    ui.colored_label(xsi::DIM(), egui::RichText::new(
                        "Open the UV Editor pane (Panes menu) to see the layout. A smaller chart angle gives more charts with less distortion.").small());
                });
            }
            NodeType::UvTransform { offset, rotate, scale } => {
                section_label(ui, "UV transform");
                section(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(xsi::LABEL(), "Offset:");
                        for a in offset.iter_mut() { ui.add(egui::DragValue::new(a).speed(0.005).max_decimals(4)); }
                    });
                    ui.horizontal(|ui| {
                        ui.colored_label(xsi::LABEL(), "Turn:");
                        ui.add(egui::DragValue::new(rotate).speed(0.5).max_decimals(2).suffix("°"));
                    });
                    ui.horizontal(|ui| {
                        ui.colored_label(xsi::LABEL(), "Scale:");
                        for a in scale.iter_mut() { ui.add(egui::DragValue::new(a).speed(0.005).max_decimals(4)); }
                    });
                });
            }
            NodeType::UvEdit { edits } => {
                section_label(ui, "UV islands");
                section(ui, |ui| {
                    ui.colored_label(xsi::DIM(), egui::RichText::new(
                        "In the UV Editor pane: click an island to select it, drag to move it. Its values appear here.").small());
                    if edits.is_empty() { ui.colored_label(xsi::LABEL(), "No island edited yet."); }
                });
                let mut remove = None;
                for (i, e) in edits.iter_mut().enumerate() {
                    ui.add_space(2.0);
                    section(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.colored_label(xsi::HEADER_TEXT(), format!("Island {}", e.island));
                            if ui.small_button("⟲ 90°").clicked() { e.rotate = (e.rotate + 90.0) % 360.0; }
                            if ui.small_button("Flip U").clicked() { e.scale[0] = -e.scale[0]; }
                            if ui.small_button("Flip V").clicked() { e.scale[1] = -e.scale[1]; }
                            if ui.small_button("🗑").on_hover_text("Remove this edit").clicked() { remove = Some(i); }
                        });
                        ui.horizontal(|ui| {
                            ui.colored_label(xsi::LABEL(), "Offset:");
                            for a in e.offset.iter_mut() { ui.add(egui::DragValue::new(a).speed(0.002).max_decimals(4)); }
                        });
                        ui.horizontal(|ui| {
                            ui.colored_label(xsi::LABEL(), "Turn:");
                            ui.add(egui::DragValue::new(&mut e.rotate).speed(0.5).max_decimals(2).suffix("°"));
                            ui.colored_label(xsi::LABEL(), "Scale:");
                            for a in e.scale.iter_mut() { ui.add(egui::DragValue::new(a).speed(0.005).max_decimals(4)); }
                        });
                    });
                }
                if let Some(i) = remove { edits.remove(i); }
            }

            // ── Modelling ────────────────────────────────────────────────────
            NodeType::EditPoly { ops, pending, edit, auto_collapse } => {
                if edit.map(|i| i >= ops.len() || ops[i].collapsed).unwrap_or(false) { *edit = None; }
                let input = io.poly_input.clone();
                // Mesh the selection under edit applies to, and the final mesh.
                let stage = input.as_ref().map(|m| eval_cached(m, ops, edit.unwrap_or(ops.len())));
                let end   = input.as_ref().map(|m| eval_cached(m, ops, ops.len()));

                section_label(ui, "Selection");
                section(ui, |ui| {
                    match *edit {
                        Some(i) => {
                            ui.colored_label(xsi::HEADER_TEXT(), format!("Selection of #{} {}", i + 1, ops[i].kind.label()));
                            ui.colored_label(xsi::DIM(), "The viewport shows the mesh before this operation.");
                            if ui.button("Done").clicked() { *edit = None; }
                        }
                        None => { ui.colored_label(xsi::HEADER_TEXT(), "Selection for the next operation"); }
                    }
                    let sel = match *edit {
                        Some(i) if i < ops.len() => &mut ops[i].selection,
                        _ => &mut *pending,
                    };
                    selection_editor(ui, sel, stage.as_deref());
                    if input.is_none() {
                        ui.colored_label(xsi::DIM(), "No mesh connected.");
                    }
                });

                ui.add_space(4.0);
                section_label(ui, "Viewport tool");
                section(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (t, label, key) in [(Tool::Select, "Select", "Q"), (Tool::Move, "Move", "W"),
                                                (Tool::Rotate, "Rotate", "E"), (Tool::Scale, "Scale", "R")] {
                            if ui.selectable_label(*io.tool == t, label).on_hover_text(format!("Key: {key}")).clicked() { *io.tool = t; }
                        }
                    });
                    ui.colored_label(xsi::DIM(), egui::RichText::new(
                        "Drag a handle in the viewport. Each drag is stored as a Transform operation.").small());
                });

                ui.add_space(4.0);
                section_label(ui, "Add operation (uses the selection above)");
                section(ui, |ui| {
                    let base = pending.level.base();
                    let (v, e, p) = (base == SubLevel::Vertex, base == SubLevel::Edge, base == SubLevel::Polygon);
                    let mut add: Option<PolyOpKind> = None;
                    let mut row = |ui: &mut egui::Ui, title: &str, items: &[(&str, bool, PolyOpKind)]| {
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(xsi::LABEL(), title);
                            for (label, on, kind) in items {
                                if ui.add_enabled(*on, egui::Button::new(*label)).clicked() { add = Some(kind.clone()); }
                            }
                        });
                    };
                    row(ui, "Polygon:", &[
                        ("Extrude", true, PolyOpKind::Extrude { height: 0.25, mode: ExtrudeMode::Group }),
                        ("Bevel",   true, PolyOpKind::Bevel { height: 0.25, outline: -0.1, mode: ExtrudeMode::Group }),
                        ("Inset",   true, PolyOpKind::Inset { amount: 0.1, by_polygon: false }),
                        ("Bridge",  p || e, PolyOpKind::Bridge),
                        ("Flip",    true, PolyOpKind::Flip),
                        ("Detach",  true, PolyOpKind::Detach),
                        ("Tessellate", true, PolyOpKind::Tessellate),
                        ("Outline", true, PolyOpKind::Outline { amount: 0.05 }),
                        ("Hinge", true, PolyOpKind::Hinge { angle: 30.0, segments: 4, edge: 0 }),
                        ("Triangulate", true, PolyOpKind::Triangulate),
                    ]);
                    row(ui, "Edge:", &[
                        ("Connect", e || v, PolyOpKind::Connect { segments: 1 }),
                        ("Remove",  e || v, PolyOpKind::Remove { clean: true }),
                        ("Cap",     true, PolyOpKind::Cap),
                        ("Chamfer", true, PolyOpKind::Chamfer { amount: 0.05 }),
                        ("Extrude", e, PolyOpKind::ExtrudeEdge { height: 0.0, width: 0.25 }),
                        ("Turn",    e, PolyOpKind::Turn),
                    ]);
                    row(ui, "Vertex:", &[
                        ("Weld",     true, PolyOpKind::Weld { threshold: 0.01 }),
                        ("Collapse", true, PolyOpKind::Collapse),
                        ("Break",    true, PolyOpKind::Break),
                        ("Extrude",  true, PolyOpKind::ExtrudeVertex { height: 0.2, width: 0.1 }),
                    ]);
                    row(ui, "Any:", &[
                        ("Delete",      true, PolyOpKind::Delete),
                        ("Transform",   true, PolyOpKind::identity_transform()),
                        ("Make planar", true, PolyOpKind::MakePlanar { axis: None }),
                        ("Relax",       true, PolyOpKind::Relax { amount: 0.5, iterations: 1, hold_border: true }),
                        ("Slice",       true, PolyOpKind::Slice { axis: 1, offset: 0.0 }),
                        ("Insert vertex", true, PolyOpKind::InsertVertex { segments: 1 }),
                    ]);
                    row(ui, "Whole mesh:", &[
                        ("Subdivide", true, PolyOpKind::Subdivide { iterations: 1 }),
                    ]);
                    if let Some(kind) = add {
                        let selection = pending.clone();
                        let level = pending.level;
                        if let Some(m) = &end {
                            let before = m.polys.len();
                            let op = PolyOp::new(selection.clone(), kind.clone());
                            *pending = match &kind {
                                // The same polygons stay selected, as in Edit Poly.
                                PolyOpKind::Extrude { .. } | PolyOpKind::Bevel { .. } | PolyOpKind::Inset { .. } => {
                                    let mask = selection.poly_mask(m);
                                    PolySelection { level, ..PolySelection::picked_polys((0..mask.len() as u32).filter(|p| mask[*p as usize]).collect()) }
                                }
                                k if k.keeps_indices() => selection.clone(),
                                // The new caps.
                                PolyOpKind::Cap => {
                                    let mut after = (**m).clone();
                                    op.apply(&mut after);
                                    PolySelection::picked_polys((before as u32..after.polys.len() as u32).collect())
                                }
                                _ => PolySelection { level, ..Default::default() },
                            };
                        }
                        push_op(ops, *auto_collapse, PolyOp::new(selection, kind));
                        *edit = None;
                    }
                });

                ui.add_space(4.0);
                section_label(ui, "Operations");
                section(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.radio_value(auto_collapse, false, "Keep live")
                            .on_hover_text("Operations stay editable until you collapse them.");
                        ui.radio_value(auto_collapse, true, "Auto-collapse")
                            .on_hover_text("Adding an operation collapses the ones before it.");
                    });
                    let live = ops.iter().filter(|op| !op.collapsed).count();
                    ui.horizontal(|ui| {
                        if ui.add_enabled(live > 0, egui::Button::new("Collapse all")).clicked() {
                            collapse_all(ops);
                            *edit = None;
                        }
                        ui.colored_label(xsi::DIM(), format!("{} live, {} collapsed", live, ops.len() - live));
                    });
                });
                ui.add_space(2.0);
                enum ListEdit { Up(usize), Down(usize), Delete(usize), Collapse(usize), Restore(usize) }
                let mut change = None;
                let count = ops.len();
                let mut i = 0;
                while i < count {
                    if ops[i].collapsed {
                        let mut j = i;
                        while j + 1 < count && ops[j + 1].collapsed { j += 1; }
                        section(ui, |ui| {
                            ui.horizontal(|ui| {
                                let n = j - i + 1;
                                ui.colored_label(xsi::DIM(), if n == 1 {
                                    format!("#{} {} (collapsed)", i + 1, ops[i].kind.label())
                                } else {
                                    format!("#{}-{}: {n} collapsed operations", i + 1, j + 1)
                                }).on_hover_text(ops[i..=j].iter().map(|o| o.kind.label()).collect::<Vec<_>>().join(", "));
                                if ui.small_button("Restore").on_hover_text("Make editable again").clicked() { change = Some(ListEdit::Restore(i)); }
                            });
                        });
                        ui.add_space(2.0);
                        i = j + 1;
                        continue;
                    }
                    let op = &mut ops[i];
                    section(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(&mut op.enabled, "");
                            ui.colored_label(xsi::HEADER_TEXT(), format!("#{} {}", i + 1, op.kind.label()));
                            if ui.selectable_label(*edit == Some(i), "🎯").on_hover_text("Edit this operation's selection").clicked() {
                                *edit = if *edit == Some(i) { None } else { Some(i) };
                            }
                            if ui.add_enabled(i > 0, egui::Button::new("⏶").small()).clicked() { change = Some(ListEdit::Up(i)); }
                            if ui.add_enabled(i + 1 < count, egui::Button::new("⏷").small()).clicked() { change = Some(ListEdit::Down(i)); }
                            if ui.small_button("✔").on_hover_text("Collapse").clicked() { change = Some(ListEdit::Collapse(i)); }
                            if ui.small_button("🗑").on_hover_text("Remove").clicked() { change = Some(ListEdit::Delete(i)); }
                        });
                        let drag = |ui: &mut egui::Ui, label: &str, v: &mut f32| {
                            ui.horizontal(|ui| {
                                ui.colored_label(xsi::LABEL(), label);
                                ui.add(egui::DragValue::new(v).speed(0.005).max_decimals(3));
                            });
                        };
                        let whole = |ui: &mut egui::Ui, label: &str, v: &mut u32, max: u32| {
                            ui.horizontal(|ui| {
                                ui.colored_label(xsi::LABEL(), label);
                                ui.add(egui::DragValue::new(v).speed(0.05).range(1..=max));
                            });
                        };
                        let mode_combo = |ui: &mut egui::Ui, mode: &mut ExtrudeMode| {
                            let name = |m: ExtrudeMode| match m {
                                ExtrudeMode::Group => "Group", ExtrudeMode::LocalNormal => "Local normal", ExtrudeMode::ByPolygon => "By polygon",
                            };
                            egui::ComboBox::from_id_source(("poly_mode", i)).selected_text(name(*mode)).show_ui(ui, |ui| {
                                for m in [ExtrudeMode::Group, ExtrudeMode::LocalNormal, ExtrudeMode::ByPolygon] {
                                    ui.selectable_value(mode, m, name(m));
                                }
                            });
                        };
                        match &mut op.kind {
                            PolyOpKind::Extrude { height, mode } => {
                                drag(ui, "Height:", height);
                                mode_combo(ui, mode);
                            }
                            PolyOpKind::Bevel { height, outline, mode } => {
                                drag(ui, "Height:", height);
                                drag(ui, "Outline:", outline);
                                mode_combo(ui, mode);
                            }
                            PolyOpKind::Inset { amount, by_polygon } => {
                                drag(ui, "Amount:", amount);
                                ui.checkbox(by_polygon, "By polygon");
                            }
                            PolyOpKind::Transform { translate, rotate, scale, falloff } => {
                                ui.horizontal(|ui| {
                                    ui.colored_label(xsi::LABEL(), "Move:");
                                    for a in translate.iter_mut() { ui.add(egui::DragValue::new(a).speed(0.01).max_decimals(3)); }
                                });
                                // Shown and edited as XYZ angles in degrees.
                                let q = Quat::from_array(*rotate).normalize();
                                let (x, y, z) = q.to_euler(EulerRot::XYZ);
                                let mut deg = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
                                let mut turned = false;
                                ui.horizontal(|ui| {
                                    ui.colored_label(xsi::LABEL(), "Rotate:");
                                    for a in deg.iter_mut() {
                                        turned |= ui.add(egui::DragValue::new(a).speed(0.5).max_decimals(2).suffix("°")).changed();
                                    }
                                });
                                if turned {
                                    *rotate = Quat::from_euler(EulerRot::XYZ, deg[0].to_radians(), deg[1].to_radians(), deg[2].to_radians()).to_array();
                                }
                                ui.horizontal(|ui| {
                                    ui.colored_label(xsi::LABEL(), "Scale:");
                                    for a in scale.iter_mut() { ui.add(egui::DragValue::new(a).speed(0.01).max_decimals(3)); }
                                });
                                ui.horizontal(|ui| {
                                    ui.colored_label(xsi::LABEL(), "Soft falloff:");
                                    ui.add(egui::DragValue::new(falloff).speed(0.01).range(0.0..=1.0e6).max_decimals(3))
                                        .on_hover_text("Soft selection: vertices within this distance of the selection follow part of the way. 0 is off");
                                });
                            }
                            PolyOpKind::Chamfer { amount } => drag(ui, "Amount:", amount),
                            PolyOpKind::ExtrudeVertex { height, width } | PolyOpKind::ExtrudeEdge { height, width } => {
                                drag(ui, "Height:", height);
                                drag(ui, "Width:", width);
                            }
                            PolyOpKind::Outline { amount } => drag(ui, "Amount:", amount),
                            PolyOpKind::Hinge { angle, segments, edge } => {
                                ui.horizontal(|ui| {
                                    ui.colored_label(xsi::LABEL(), "Angle:");
                                    ui.add(egui::DragValue::new(angle).speed(0.5).range(-360.0..=360.0).suffix("°"));
                                });
                                whole(ui, "Segments:", segments, 64);
                                ui.horizontal(|ui| {
                                    ui.colored_label(xsi::LABEL(), "Hinge edge:");
                                    ui.add(egui::DragValue::new(edge).speed(0.05).range(0..=9999))
                                        .on_hover_text("Which edge of the selection's outline is the hinge");
                                });
                            }
                            PolyOpKind::Slice { axis, offset } => {
                                ui.horizontal(|ui| {
                                    ui.colored_label(xsi::LABEL(), "Across:");
                                    ui.selectable_value(axis, 0, "X");
                                    ui.selectable_value(axis, 1, "Y");
                                    ui.selectable_value(axis, 2, "Z");
                                });
                                drag(ui, "At:", offset);
                            }
                            PolyOpKind::InsertVertex { segments } => whole(ui, "Per edge:", segments, 64),
                            PolyOpKind::Remove { clean } => {
                                ui.checkbox(clean, "Also remove leftover vertices");
                            }
                            PolyOpKind::Weld { threshold } => drag(ui, "Threshold:", threshold),
                            PolyOpKind::Connect { segments } => whole(ui, "Segments:", segments, 64),
                            PolyOpKind::MakePlanar { axis } => {
                                ui.horizontal(|ui| {
                                    ui.selectable_value(axis, None, "Best fit");
                                    ui.selectable_value(axis, Some(0), "X");
                                    ui.selectable_value(axis, Some(1), "Y");
                                    ui.selectable_value(axis, Some(2), "Z");
                                });
                            }
                            PolyOpKind::Relax { amount, iterations, hold_border } => {
                                drag(ui, "Amount:", amount);
                                whole(ui, "Iterations:", iterations, 200);
                                ui.checkbox(hold_border, "Hold open borders");
                            }
                            PolyOpKind::Subdivide { iterations } => whole(ui, "Iterations:", iterations, 4),
                            PolyOpKind::Delete | PolyOpKind::Collapse | PolyOpKind::Cap | PolyOpKind::Bridge
                            | PolyOpKind::Detach | PolyOpKind::Break | PolyOpKind::Flip | PolyOpKind::Tessellate
                            | PolyOpKind::Triangulate | PolyOpKind::Turn => {}
                        }
                    });
                    ui.add_space(2.0);
                    i += 1;
                }
                match change {
                    Some(ListEdit::Up(i))       => { ops.swap(i, i - 1); *edit = None; }
                    Some(ListEdit::Down(i))     => { ops.swap(i, i + 1); *edit = None; }
                    Some(ListEdit::Delete(i))   => { ops.remove(i); *edit = None; }
                    Some(ListEdit::Collapse(i)) => { collapse_op(ops, i); *edit = None; }
                    Some(ListEdit::Restore(i))  => { restore_run(ops, i); }
                    None => {}
                }
                if let Some(m) = &end {
                    ui.colored_label(xsi::DIM(), format!("Result: {} vertices, {} polygons", m.verts.len(), m.polys.len()));
                }
            }
        }
    }
    if resync { graph.sync_sockets(sel_id); }

    ui.add_space(6.0);
    ui.colored_label(xsi::DIM(), format!("「{}」", node_name));
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Sub-object level, where the selection comes from, and its modifiers.
fn selection_editor(ui: &mut egui::Ui, sel: &mut PolySelection, mesh: Option<&PolyMesh>) {
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut sel.level, SubLevel::Vertex,  "Vertex");
        ui.selectable_value(&mut sel.level, SubLevel::Edge,    "Edge");
        ui.selectable_value(&mut sel.level, SubLevel::Border,  "Border");
        ui.selectable_value(&mut sel.level, SubLevel::Polygon, "Polygon");
        ui.selectable_value(&mut sel.level, SubLevel::Element, "Element");
    });

    let name = |s: &SelSource| match s {
        SelSource::Picked          => "Picked in viewport",
        SelSource::All             => "All",
        SelSource::ByNormal { .. } => "By normal",
        SelSource::InBox { .. }    => "In box",
    };
    egui::ComboBox::from_id_source("poly_sel_source").selected_text(name(&sel.source)).show_ui(ui, |ui| {
        if ui.selectable_label(sel.source == SelSource::Picked, "Picked in viewport").clicked() { sel.source = SelSource::Picked; }
        if ui.selectable_label(sel.source == SelSource::All, "All").clicked() { sel.source = SelSource::All; }
        if ui.selectable_label(matches!(sel.source, SelSource::ByNormal { .. }), "By normal").clicked()
            && !matches!(sel.source, SelSource::ByNormal { .. })
        {
            sel.source = SelSource::ByNormal { dir: [0.0, 1.0, 0.0], angle: 45.0 };
        }
        if ui.selectable_label(matches!(sel.source, SelSource::InBox { .. }), "In box").clicked()
            && !matches!(sel.source, SelSource::InBox { .. })
        {
            // Start with a box around the whole mesh.
            let (mut lo, mut hi) = ([-1.0f32; 3], [1.0f32; 3]);
            if let Some(m) = mesh.filter(|m| !m.verts.is_empty()) {
                lo = [f32::MAX; 3];
                hi = [f32::MIN; 3];
                for v in &m.verts { for a in 0..3 { lo[a] = lo[a].min(v[a]); hi[a] = hi[a].max(v[a]); } }
            }
            sel.source = SelSource::InBox { min: lo, max: hi };
        }
    });

    match &mut sel.source {
        SelSource::ByNormal { dir, angle } => {
            ui.horizontal(|ui| {
                for (label, d) in [("+X", [1.0, 0.0, 0.0]), ("-X", [-1.0, 0.0, 0.0]), ("+Y", [0.0, 1.0, 0.0]),
                                   ("-Y", [0.0, -1.0, 0.0]), ("+Z", [0.0, 0.0, 1.0]), ("-Z", [0.0, 0.0, -1.0])] {
                    if ui.selectable_label(*dir == d, label).clicked() { *dir = d; }
                }
            });
            ui.horizontal(|ui| {
                ui.colored_label(xsi::LABEL(), "Within:");
                ui.add(egui::DragValue::new(angle).speed(0.5).range(0.0..=180.0).suffix("°"));
            });
        }
        SelSource::InBox { min, max } => {
            for (label, v) in [("Min", min), ("Max", max)] {
                ui.horizontal(|ui| {
                    ui.colored_label(xsi::LABEL(), label);
                    for a in v.iter_mut() { ui.add(egui::DragValue::new(a).speed(0.01).max_decimals(3)); }
                });
            }
        }
        SelSource::Picked => {
            ui.colored_label(xsi::DIM(), egui::RichText::new(
                "Click or drag a box in the viewport. Ctrl adds, Shift removes.").small());
        }
        SelSource::All => {}
    }

    ui.horizontal(|ui| {
        if ui.button("Shrink").clicked() { sel.grow -= 1; }
        ui.colored_label(xsi::LABEL(), format!("{:+}", sel.grow));
        if ui.button("Grow").clicked() { sel.grow += 1; }
        ui.checkbox(&mut sel.invert, "Invert");
        if ui.button("Clear").clicked() {
            let level = sel.level;
            *sel = PolySelection { level, ..Default::default() };
        }
    });

    if let (Some(m), SubLevel::Edge) = (mesh, sel.level) {
        ui.horizontal(|ui| {
            if ui.button("Loop").on_hover_text("Extend the selected edges along their loops").clicked() { sel.expand_edges(m, false); }
            if ui.button("Ring").on_hover_text("Extend the selected edges across their rings").clicked() { sel.expand_edges(m, true); }
        });
    }

    if let Some(m) = mesh {
        let what = match sel.level.base() { SubLevel::Vertex => "vertices", SubLevel::Polygon => "polygons", _ => "edges" };
        let polys = sel.poly_mask(m).iter().filter(|s| **s).count();
        let text = if sel.level.base() == SubLevel::Polygon {
            format!("{polys} polygons selected")
        } else {
            format!("{} {what} selected, covering {polys} polygons", sel.count(m))
        };
        ui.colored_label(xsi::LABEL(), text);
    }
}

/// Text field for a path with a folder button that opens the file browser.
fn path_row(
    ui: &mut egui::Ui, value: &mut String, hint: &str, io: &mut PanelIo,
    node: NodeId, mode: BrowseMode, title: &str, exts: &[&str],
) {
    ui.horizontal(|ui| {
        if ui.button("📂").on_hover_text("Browse").clicked() {
            io.browser.open(BrowseTarget::Node(node), mode, title, exts, "");
        }
        ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY));
    });
}

/// Progress and results of the last write.
fn batch_log(ui: &mut egui::Ui, batch: &BatchState) {
    let p = batch.0.lock().unwrap();
    if p.log.is_empty() && !p.running { return; }
    ui.add_space(4.0);
    section(ui, |ui| {
        if p.running {
            ui.add(egui::ProgressBar::new(p.done as f32 / p.total.max(1) as f32)
                .text(format!("writing {} / {}", p.done, p.total)));
        }
        egui::ScrollArea::vertical().id_source("batch_log").max_height(260.0).stick_to_bottom(true).show(ui, |ui| {
            for line in &p.log {
                let col = if line.starts_with("FAILED") { egui::Color32::from_rgb(230, 150, 150) } else { xsi::LABEL() };
                ui.colored_label(col, egui::RichText::new(line).small());
            }
        });
    });
}

fn rate_combo(ui: &mut egui::Ui, id: &str, num: &mut u32, den: &mut u32) {
    let cur = FrameRate::new(*num, *den);
    ui.horizontal(|ui| {
        ui.colored_label(xsi::LABEL(), "Rate:");
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
            Some(i) => { ui.colored_label(xsi::DIM(), format!("In:   {}", line(i))); }
            None    => { ui.colored_label(xsi::DIM(), "In:   no clip connected"); }
        }
        if let Some(o) = &anim.output {
            ui.colored_label(xsi::LABEL(), format!("Out:  {}", line(o)));
        }
    });
}

fn section(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(xsi::SECTION_BG())
        .inner_margin(egui::vec2(8.0, 6.0))
        .rounding(4.0)
        .show(ui, add);
}

fn section_label(ui: &mut egui::Ui, label: &str) {
    ui.colored_label(xsi::DIM(), egui::RichText::new(label).size(10.0).strong());
    ui.add_space(2.0);
}

fn labeled_slider(ui: &mut egui::Ui, label: &str, val: &mut f32, range: std::ops::RangeInclusive<f32>) {
    ui.horizontal(|ui| {
        ui.colored_label(xsi::LABEL(), format!("{label}:"));
        ui.add(egui::Slider::new(val, range));
    });
}

fn labeled_slider_u32(ui: &mut egui::Ui, label: &str, val: &mut u32, range: std::ops::RangeInclusive<u32>) {
    ui.horizontal(|ui| {
        ui.colored_label(xsi::LABEL(), format!("{label}:"));
        ui.add(egui::Slider::new(val, range));
    });
}

fn drag_vec3(ui: &mut egui::Ui, v: &mut Vec3, speed: f64) {
    ui.horizontal(|ui| {
        ui.colored_label(xsi::LABEL(), "X:"); ui.add(egui::DragValue::new(&mut v.x).speed(speed));
        ui.colored_label(xsi::LABEL(), "Y:"); ui.add(egui::DragValue::new(&mut v.y).speed(speed));
        ui.colored_label(xsi::LABEL(), "Z:"); ui.add(egui::DragValue::new(&mut v.z).speed(speed));
    });
}