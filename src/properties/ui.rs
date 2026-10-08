//! The Properties pane: the parameters of the selected node.
//!
//! Every node is laid out the same way (see `form`): a first group with the
//! node's type and name, then its parameters in titled groups of aligned
//! rows, then what came in and what goes out. The same words mean the same
//! thing everywhere: Translate, Rotate and Scale for every transform, Path
//! for every file, In and Out for what passes through.

use bevy::prelude::Vec3;
use bevy_egui::egui;
use std::sync::Arc;
use crate::types::{BodyView, NodeId, NodeType, RetimeMode, SplitPick, node_type_icon, node_type_label};
use crate::core::anim::{AnimData, FrameRate, PoseEdit, RATE_PRESETS};
use crate::file_browser::{BrowseMode, BrowseTarget, FileBrowser};
use crate::batch::{self, BatchState};
use crate::core::manip::Tool;
use crate::core::poly::{collapse_all, collapse_op, eval_cached, push_op, restore_run, ExtrudeMode, PolyMesh, PolyOp, PolyOpKind, PolySelection, SelSource, SubLevel};
use bevy::math::{EulerRot, Quat};
use crate::node_graph::NodeGraphState;
use crate::scene_graph::SceneGraph;
use crate::ice::{SubnetStore, GraphNavigation};
use crate::ice::ui::draw_subnet_node_properties;

use super::form::{self, group, group_with, ink, Rows, Status};

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
        .fill(ink::PANEL_BG())
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

const FBX_EXTS: &[&str] = &["fbx", crate::packed::CLIP_EXT];

pub fn draw_properties(
    ui:     &mut egui::Ui,
    graph:  &mut NodeGraphState,
    _scene: &SceneGraph,
    anim:   &AnimContext,
    io:     &mut PanelIo,
) {
    let Some(sel_id) = graph.selected_node else {
        ui.colored_label(ink::DIM(), "Select a node to see its parameters.");
        return;
    };

    // Facts about the graph the parameter widgets need, gathered before the
    // node is borrowed for editing.
    let all_writers  = batch::write_nodes(graph);
    let batch_files  = batch::file_count(graph, &batch::upstream_folder_loaders(graph, &[sel_id]));
    let mut resync   = false;
    // Packed primitives coming into the node: their paths, and which are picked.
    // Asked only for the nodes that list them: it cooks what is upstream.
    let packed_in: Vec<(String, bool)> = graph.nodes.iter().find(|n| n.id == sel_id)
        .filter(|n| matches!(n.node_type, NodeType::PickPrims { .. } | NodeType::PrunePrims { .. } | NodeType::UnpackPrims))
        .and_then(|n| n.inputs.first()).and_then(|i| i.connected_output)
        .and_then(|(src, out)| {
            let pass = |_: crate::types::SubnetId, m: &crate::types::MeshData, _: Option<&crate::types::MeshData>| m.clone();
            graph.eval_packed(src, out, &pass)
        }).unwrap_or_default();
    let prim_paths: Vec<String> = packed_in.iter().map(|p| p.0.clone()).collect();
    // What reaches a Body Collide node: its clip and its collider.
    let collide_in = match graph.nodes.iter().find(|n| n.id == sel_id).map(|n| &n.node_type) {
        Some(NodeType::Ragdoll { .. }) => graph.ragdoll_inputs(sel_id),
        _ => (None, None),
    };

    let Some(node) = graph.nodes.iter_mut().find(|n| n.id == sel_id) else { return };

    // ── The node itself ──────────────────────────────────────────────────────
    let title = format!("{}  {}", node_type_icon(&node.node_type), node_type_label(&node.node_type));
    group(ui, &title, None, |f| { f.text("Name", None, &mut node.name, ""); });

    match &mut node.node_type {
        // ── Create ───────────────────────────────────────────────────────────
        NodeType::CreateCube { size } => group(ui, "Shape", None, |f| {
            f.slider("Size", None, size, 0.1..=5.0, " m");
        }),
        NodeType::CreateSphere { radius, segments } => group(ui, "Shape", None, |f| {
            f.slider("Radius", None, radius, 0.1..=3.0, " m");
            f.slider_u32("Segments", None, segments, 4..=64, "");
        }),
        NodeType::CreateGrid { rows, cols, size } => group(ui, "Shape", None, |f| {
            f.slider_u32("Rows", None, rows, 1..=100, "");
            f.slider_u32("Columns", None, cols, 1..=100, "");
            f.slider("Size", None, size, 0.1..=20.0, " m");
        }),
        NodeType::TestClip { seconds, fps_num, fps_den } => group(ui, "Clip", Some("A procedural walk on the spot, starting at 01:00:00:00"), |f| {
            f.slider("Length", None, seconds, 0.5..=60.0, " s");
            rate_row(f, fps_num, fps_den);
        }),

        // ── Files ────────────────────────────────────────────────────────────
        NodeType::LoadUsd { path } => {
            group(ui, "File", Some("A USD file (.usda, .usdc, .usdz), read as packed primitives"), |f| {
                path_row(f, path, "/path/to/file.usdz", io, sel_id, BrowseMode::File, "Open USD", &["usda", "usdc", "usdz", "usd"]);
                if path.is_empty() { f.status(Status::Info, "No file set"); }
                else if std::path::Path::new(path).exists() { f.status(Status::Ok, "Found"); }
                else { f.status(Status::Bad, "Not found"); }
            });
            if !path.is_empty() { usd_summary(ui, path); }
        }
        NodeType::LoadFbx { path, take } => {
            group(ui, "File", Some("A skeleton and one take, baked per frame, with the mesh skinned to it. Read Y up, in metres; written back in the file's own axes and unit"), |f| {
                path_row(f, path, "/path/to/take.fbx", io, sel_id, BrowseMode::File, "Open FBX", FBX_EXTS);
                take_rows(f, crate::fbx_loader::load_fbx_cached(path, *take), take);
            });
            clip_summary(ui, anim, false);
        }
        NodeType::LoadFbxDir { dir, index, take } => {
            let files = crate::fbx_loader::list_fbx(dir);
            group(ui, "Folder", Some("One FBX of the folder at a time. Write FBX can run over all of them"), |f| {
                path_row(f, dir, "/path/to/folder", io, sel_id, BrowseMode::Folder, "Choose FBX folder", &["fbx"]);
                if files.is_empty() {
                    f.status(if dir.is_empty() { Status::Info } else { Status::Bad }, if dir.is_empty() { "No folder set" } else { "No .fbx files in this folder" });
                    return;
                }
                let last = files.len() as u32 - 1;
                *index = (*index).min(last);
                let name = |i: usize| files[i].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                f.row("File", None, |ui| {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(*index > 0, egui::Button::new("◀")).clicked() { *index -= 1; }
                        if ui.add_enabled(*index < last, egui::Button::new("▶")).clicked() { *index += 1; }
                        egui::ComboBox::from_id_source("fbx_dir_file")
                            .width(ui.available_width() - 8.0)
                            .selected_text(format!("{} of {}  {}", *index + 1, files.len(), name(*index as usize)))
                            .show_ui(ui, |ui| {
                                for i in 0..files.len() { ui.selectable_value(index, i as u32, format!("{}  {}", i + 1, name(i))); }
                            });
                    });
                });
                take_rows(f, crate::fbx_loader::load_fbx_cached(&files[*index as usize].to_string_lossy(), *take), take);
            });
            clip_summary(ui, anim, false);
        }
        NodeType::LoadFbxMesh { path } => group(ui, "File", Some("Every mesh of an FBX file, where it stands, as packed primitives. A set for Body Collide to keep a character out of"), |f| {
            path_row(f, path, "/path/to/set.fbx", io, sel_id, BrowseMode::File, "Open FBX", &["fbx", crate::packed::MESH_EXT]);
            match crate::fbx_loader::load_meshes_cached(path) {
                Ok(meshes) => {
                    let tris: usize = meshes.iter().map(|m| m.1.indices.len() / 3).sum();
                    f.status(Status::Ok, format!("{} mesh{}, {} triangles", meshes.len(), if meshes.len() == 1 { "" } else { "es" }, tris));
                }
                Err(e) if path.is_empty() => { let _ = e; f.status(Status::Info, "No file set"); }
                Err(e) => f.status(Status::Bad, e),
            }
        }),
        NodeType::WriteFbx { path, mesh } => {
            group(ui, "File", Some("Tokens in the path: {dir} source folder, {file} source file, {char} character, {take} take name"), |f| {
                path_row(f, path, crate::types::DEFAULT_WRITE_PATH, io, sel_id, BrowseMode::Folder, "Choose output folder", &["fbx"]);
                f.toggle("Mesh", Some("Also write the skinned mesh, its skin and bind pose. Off: the skeleton and its motion only, for importing onto a skeleton the engine already has"), mesh);
                match &anim.output {
                    Some(c) => {
                        f.value("Writes", crate::fbx_writer::resolve_path(path, c).display().to_string());
                        f.value("Content", format!("{} joints, {}{}", c.joints.len(),
                            if c.frames > 1 { format!("{} frames", c.frames) } else { "one pose".into() },
                            match (&c.skin, *mesh) { (Some(s), true) => format!(", mesh of {} vertices", s.positions.len()), _ => String::new() }));
                        f.value("Space", space_line(c));
                    }
                    None => f.status(Status::Info, "No clip connected"),
                }
            });
            let running = io.batch.0.lock().unwrap().running;
            group(ui, "Write", None, |f| {
                f.enabled(!running, |f| {
                    f.buttons("This node", None, |ui| {
                        if ui.button("This file").clicked() { io.action = Some(PanelAction::Write { targets: vec![sel_id], all_files: false }); }
                        if ui.add_enabled(batch_files > 0, egui::Button::new(format!("Whole folder ({batch_files})"))).clicked() {
                            io.action = Some(PanelAction::Write { targets: vec![sel_id], all_files: true });
                        }
                    });
                    f.buttons(&format!("All {} Write nodes", all_writers.len()), None, |ui| {
                        if ui.button("This file").clicked() { io.action = Some(PanelAction::Write { targets: all_writers.clone(), all_files: false }); }
                        if ui.add_enabled(batch_files > 0, egui::Button::new(format!("Whole folder ({batch_files})"))).clicked() {
                            io.action = Some(PanelAction::Write { targets: all_writers.clone(), all_files: true });
                        }
                    });
                });
            });
            batch_log(ui, io.batch);
        }

        // ── Modify ───────────────────────────────────────────────────────────
        NodeType::Transform { translation, rotation, scale } => {
            let clip = anim.input.is_some();
            let mut reset = false;
            group_with(ui, "Transform",
                Some("Moves whatever comes in: a mesh, packed primitives (the picked ones, or all of them), or a clip. A clip moves by its top joints, so its skinned mesh moves with it once. Scale, then rotate X, Y, Z, then translate"),
                |ui| { if ui.small_button("Reset").clicked() { reset = true; } },
                |f| {
                    let mut t = translation.to_array();
                    if f.xyz("Translate", None, &mut t, 0.01, "") { *translation = Vec3::from_array(t); }
                    let mut deg = [rotation.x.to_degrees(), rotation.y.to_degrees(), rotation.z.to_degrees()];
                    if f.xyz("Rotate", None, &mut deg, 0.5, "°") { *rotation = Vec3::new(deg[0].to_radians(), deg[1].to_radians(), deg[2].to_radians()); }
                    if clip {
                        let mut s = scale.x;
                        if f.drag("Scale", Some("A skeleton scales the same on every axis"), &mut s, 0.01, "").changed() { *scale = Vec3::splat(s); }
                    } else {
                        let mut s = scale.to_array();
                        if f.xyz("Scale", None, &mut s, 0.01, "") { *scale = Vec3::from_array(s); }
                    }
                });
            if reset { *translation = Vec3::ZERO; *rotation = Vec3::ZERO; *scale = Vec3::ONE; }
            if clip { clip_summary(ui, anim, true); }
        }
        NodeType::Merge => group(ui, "Inputs", None, |f| { f.status(Status::Info, "A and B, as one mesh"); }),
        NodeType::ScatterPoints { count, seed } => group(ui, "Points", None, |f| {
            f.slider_u32("Count", None, count, 1..=10_000, "");
            f.slider_u32("Seed", Some("Another seed, another spread"), seed, 0..=9_999, "");
        }),
        NodeType::CopyToPoints => group(ui, "Inputs", None, |f| {
            f.value("Template", "First input: the mesh copied");
            f.value("Points", "Second input: where the copies go");
        }),
        NodeType::Subnet { .. } => group(ui, "Subnet", None, |f| { f.status(Status::Info, "Double-click the node to go inside"); }),
        NodeType::Output => group(ui, "Output", None, |f| { f.status(Status::Info, "What reaches this node is the scene"); }),

        // ── Primitives ───────────────────────────────────────────────────────
        NodeType::PickPrims { pattern } => group(ui, "Pick", Some("The nodes after this one work on the picked primitives only and pass the others through: Edit Poly, Transform and the UV nodes all do"), |f| {
            pattern_rows(f, "pick_prims", "Paths", pattern, &prim_paths, false);
            if prim_paths.is_empty() { f.status(Status::Info, "No packed primitives coming in"); }
        }),
        NodeType::PrunePrims { pattern, keep } => group(ui, "Prune", None, |f| {
            pattern_rows(f, "prune_prims", "Paths", pattern, &prim_paths, false);
            f.choice("Matches", None, keep, &[(false, "Remove"), (true, "Keep only")]);
            if prim_paths.is_empty() { f.status(Status::Info, "No packed primitives coming in"); }
        }),
        NodeType::UnpackPrims => group(ui, "Unpack", Some("Nodes that need one mesh do this on their own. Unpack makes it explicit, or drops a pick"), |f| {
            f.value("In", format!("{} packed primitives", prim_paths.len()));
            f.value("Out", "One mesh");
        }),

        // ── UV ───────────────────────────────────────────────────────────────
        NodeType::UvUnwrap { method, angle, margin, axis, tiles } => {
            use crate::core::uv::UvMethod;
            group(ui, "Unwrap", Some("The UV Editor pane shows the layout"), |f| {
                f.choice("Method", Some("Conformal: charts by normal angle, each flattened with least squares conformal maps and packed at equal density"),
                    method, &[(UvMethod::Conformal, "Conformal"), (UvMethod::Box, "Box"), (UvMethod::Planar, "Planar")]);
                match method {
                    UvMethod::Conformal => { f.slider("Chart angle", Some("Smaller: more charts, less distortion"), angle, 5.0..=89.0, "°"); }
                    UvMethod::Planar => { f.choice("Along", None, axis, &[(0, "X"), (1, "Y"), (2, "Z")]); }
                    UvMethod::Box => {}
                }
                f.slider("Margin", Some("Gap between charts, in UV space"), margin, 0.0..=0.1, "");
                f.drag_u32("UDIM tiles", Some("The separate pieces of the mesh spread over this many tiles. Pieces close together share one; the most surface goes to 1001"), tiles, 1..=100);
                if *tiles > 1 { f.value("", format!("1001 to {}", 1000 + *tiles)); }
            });
        }
        NodeType::UvTransform { offset, rotate, scale } => group(ui, "Transform", None, |f| {
            f.uv("Translate", None, offset, 0.005);
            f.drag("Rotate", None, rotate, 0.5, "°");
            f.uv("Scale", None, scale, 0.005);
        }),
        NodeType::UvEdit { edits } => {
            group(ui, "Islands", Some("In the UV Editor pane: click an island to select it, drag to move it"), |f| {
                if edits.is_empty() { f.status(Status::Info, "No island edited yet"); }
                else { f.value("Edited", format!("{} island{}", edits.len(), if edits.len() == 1 { "" } else { "s" })); }
            });
            let mut remove = None;
            for (i, e) in edits.iter_mut().enumerate() {
                let mut turn = false; let (mut flip_u, mut flip_v) = (false, false);
                group_with(ui, &format!("Island {}", e.island), None, |ui| {
                    if ui.small_button("🗑").on_hover_text("Remove this edit").clicked() { remove = Some(i); }
                    if ui.small_button("Flip V").clicked() { flip_v = true; }
                    if ui.small_button("Flip U").clicked() { flip_u = true; }
                    if ui.small_button("⟲ 90°").clicked() { turn = true; }
                }, |f| {
                    f.uv("Translate", None, &mut e.offset, 0.002);
                    f.drag("Rotate", None, &mut e.rotate, 0.5, "°");
                    f.uv("Scale", None, &mut e.scale, 0.005);
                });
                if turn { e.rotate = (e.rotate + 90.0) % 360.0; }
                if flip_u { e.scale[0] = -e.scale[0]; }
                if flip_v { e.scale[1] = -e.scale[1]; }
            }
            if let Some(i) = remove { edits.remove(i); }
        }

        // ── Animation ────────────────────────────────────────────────────────
        NodeType::RenameJoints { find, replace, strip_namespace, prefix } => {
            let names: Vec<String> = anim.input.as_ref().map(|c| c.joints.iter().map(|j| j.name.clone()).collect()).unwrap_or_default();
            group(ui, "Rename", Some("Find is a regular expression, case ignored. Replace may use $1, $2 for its groups"), |f| {
                f.toggle("Strip namespace", Some("Remove the text up to the last ':'"), strip_namespace);
                pattern_rows(f, "rename_find", "Find", find, &names, true);
                f.text("Replace", None, replace, "");
                f.text("Prefix", None, prefix, "");
            });
            if let (Some(i), Some(o)) = (&anim.input, &anim.output) {
                group(ui, "Result", None, |f| f.wide(|ui| {
                    egui::ScrollArea::vertical().id_source("rename_preview").max_height(220.0).show(ui, |ui| {
                        for (a, b) in i.joints.iter().zip(o.joints.iter()) {
                            let col = if a.name == b.name { ink::DIM() } else { ink::VALUE() };
                            ui.colored_label(col, format!("{}  >  {}", a.name, b.name));
                        }
                    });
                }));
            }
        }
        NodeType::TrimClip { head, tail } => {
            group(ui, "Trim", Some("Frames cut from the start and from the end of the clip"), |f| {
                let max = anim.input.as_ref().map(|i| i.frames.saturating_sub(1) as u32).unwrap_or(100_000);
                f.drag_u32("Cut at start", None, head, 0..=max);
                f.drag_u32("Cut at end", None, tail, 0..=max);
                if let Some(i) = &anim.input {
                    let playhead = i.frame_at(anim.time).clamp(i.start_frame, i.end_frame());
                    f.buttons("Playhead", None, |ui| {
                        if ui.button("Start here").clicked() { *head = (playhead - i.start_frame) as u32; *tail = (*tail).min(max - *head); }
                        if ui.button("End here").clicked() { *tail = (i.end_frame() - playhead) as u32; *head = (*head).min(max - *tail); }
                        if ui.button("Reset").clicked() { *head = 0; *tail = 0; }
                    });
                }
            });
            clip_summary(ui, anim, true);
        }
        NodeType::Retime { fps_num, fps_den, mode } => {
            group(ui, "Retime", None, |f| {
                rate_row(f, fps_num, fps_den);
                f.choice("Mode", Some("Resample keeps the duration and makes new frames. Reinterpret keeps the frames and changes the speed"),
                    mode, &[(RetimeMode::Resample, "Resample"), (RetimeMode::Reinterpret, "Reinterpret")]);
            });
            clip_summary(ui, anim, true);
        }
        NodeType::SetTimecode { hours, minutes, seconds, frames, drop_frame } => {
            group(ui, "Timecode", None, |f| {
                let tb = anim.input.as_ref().map(|i| i.rate.timebase() as u32).unwrap_or(120);
                f.row("Start", Some("Hours : minutes : seconds : frames"), |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(hours).range(0..=23));
                        ui.label(":");
                        ui.add(egui::DragValue::new(minutes).range(0..=59));
                        ui.label(":");
                        ui.add(egui::DragValue::new(seconds).range(0..=59));
                        ui.label(":");
                        ui.add(egui::DragValue::new(frames).range(0..=tb.saturating_sub(1)));
                    });
                });
                let can_drop = anim.input.as_ref().map(|i| i.rate.supports_drop_frame()).unwrap_or(true);
                f.enabled(can_drop, |f| { f.toggle("Drop frame", Some("29.97 and 59.94 only"), drop_frame); });
            });
            clip_summary(ui, anim, true);
        }
        NodeType::TimeWarp { speed, reverse } => {
            group(ui, "Time warp", None, |f| {
                f.drag("Speed", None, speed, 0.01, " ×");
                *speed = speed.clamp(0.05, 20.0);
                f.toggle("Reverse", None, reverse);
            });
            clip_summary(ui, anim, true);
        }
        NodeType::BlendClips { blend, align } => {
            group(ui, "Blend", Some("The first input, then the second. Joints are matched by name"), |f| {
                f.slider_u32("Blend", Some("Frames over which one clip becomes the other"), blend, 0..=240, "");
                f.toggle("Align", Some("Start the second clip where the first one ends"), align);
            });
            clip_summary(ui, anim, true);
        }
        NodeType::LoopClip { blend } => {
            group(ui, "Loop", Some("The end of the clip eased into the pose of its first frame. Travel is kept"), |f| {
                f.slider_u32("Blend", Some("Frames over which the end is eased"), blend, 0..=240, "");
            });
            clip_summary(ui, anim, true);
        }
        NodeType::MirrorClip => {
            group(ui, "Mirror", Some("Joints pair up by name: Left / Right, L_ / R_, _L / _R"), |f| {
                f.status(Status::Info, "Left and right swap, the motion is reflected");
            });
            clip_summary(ui, anim, true);
        }
        NodeType::SmoothClip { radius, amount, translations } => {
            group(ui, "Smooth", None, |f| {
                f.slider_u32("Radius", Some("Frames on each side"), radius, 0..=30, "");
                f.slider("Amount", None, amount, 0.0..=1.0, "");
                f.toggle("Positions too", None, translations);
            });
            clip_summary(ui, anim, true);
        }
        NodeType::InPlace { keep_height, to_root } => {
            group(ui, "In place", None, |f| {
                f.toggle("Keep height", Some("Keep the up and down motion"), keep_height);
                f.toggle("Root motion", Some("The joint above the hips carries the travel, the hips stay under it"), to_root);
            });
            clip_summary(ui, anim, true);
        }
        NodeType::FloorClip { height } => {
            group(ui, "Floor", None, |f| {
                f.drag("Height", None, height, 0.005, " m");
                if let Some(i) = &anim.input { f.value("Lowest now", format!("{:.3} m", i.lowest_point())); }
            });
            clip_summary(ui, anim, true);
        }
        NodeType::PruneJoints { words } => {
            let names: Vec<String> = anim.input.as_ref().map(|c| c.joints.iter().map(|j| j.name.clone()).collect()).unwrap_or_default();
            group(ui, "Prune", Some("Each matching joint goes, with everything below it"), |f| {
                pattern_rows(f, "prune_words", "Joints", words, &names, false);
                if let (Some(i), Some(o)) = (&anim.input, &anim.output) { f.value("Joints", format!("{} in, {} out", i.joints.len(), o.joints.len())); }
            });
        }
        NodeType::TransformClip { .. } | NodeType::Calamari { .. } => group(ui, "Older node", None, |f| {
            f.status(Status::Info, "Open the graph again to bring this node up to date");
        }),

        // ── Mocap ────────────────────────────────────────────────────────────
        NodeType::SplitSkeleton { picks } => {
            let input = anim.input.clone();
            let chars: Vec<(usize, String)> = input.as_ref()
                .map(|c| c.character_roots().into_iter().map(|r| (r, c.character_name(r))).collect())
                .unwrap_or_default();
            let (mut detect, mut add) = (false, false);
            let mut removed = false;
            group_with(ui, "Outputs", Some("One output per character"), |ui| {
                if ui.add_enabled(!chars.is_empty(), egui::Button::new("Detect").small())
                    .on_hover_text("One output per character found in the incoming clip").clicked() { detect = true; }
                if ui.small_button("+").on_hover_text("Add an output").clicked() { add = true; }
            }, |f| {
                let mut remove = None;
                for (n, pick) in picks.iter_mut().enumerate() {
                    f.row(&format!("Output {}", n + 1), None, |ui| {
                        ui.horizontal(|ui| {
                            let text = match &*pick {
                                SplitPick::Character(i) => match chars.get(*i as usize) {
                                    Some((_, name)) => format!("Character {}: {name}", i + 1),
                                    None            => format!("Character {} (not in this file)", i + 1),
                                },
                                SplitPick::Joint(name) => format!("Joint {name}"),
                            };
                            egui::ComboBox::from_id_source(("split_pick", n))
                                .width(ui.available_width() - 30.0)
                                .selected_text(text)
                                .show_ui(ui, |ui| {
                                    ui.weak("Characters, by position");
                                    for (i, (_, name)) in chars.iter().enumerate() {
                                        ui.selectable_value(pick, SplitPick::Character(i as u32), format!("Character {}: {name}", i + 1));
                                    }
                                    if let Some(c) = &input {
                                        ui.separator();
                                        ui.weak("Any joint, by name");
                                        for j in &c.joints { ui.selectable_value(pick, SplitPick::Joint(j.name.clone()), &j.name); }
                                    }
                                });
                            if ui.small_button("🗑").on_hover_text("Remove this output").clicked() { remove = Some(n); }
                        });
                    });
                }
                if let Some(n) = remove { picks.remove(n); removed = true; }
            });
            if detect { *picks = (0..chars.len() as u32).map(SplitPick::Character).collect(); }
            if add { picks.push(SplitPick::Character(picks.len() as u32)); }
            resync |= detect || add || removed;
            group(ui, "Characters in", None, |f| match &input {
                None => f.status(Status::Info, "No clip connected"),
                Some(_) if chars.is_empty() => f.status(Status::Info, "None found in the incoming clip"),
                Some(c) => for (i, (root, name)) in chars.iter().enumerate() {
                    f.value(&format!("{}", i + 1), format!("{name}: {} joints from \"{}\"", c.subtree(*root).len(), c.joints[*root].name));
                },
            });
        }
        NodeType::Retarget => {
            group(ui, "Retarget", Some("The motion of the first input on the skeleton of the second. Joints are matched by name, ignoring namespaces and the prefix each skeleton shares. The skeletons may rest in different poses: bones are lined up at rest first"), |f| {
                match (&anim.input, &anim.output) {
                    (Some(i), Some(o)) => { f.value("Motion", format!("{} joints", i.joints.len())); f.value("Result", format!("{} joints", o.joints.len())); }
                    _ => f.status(Status::Info, "Connect a clip to both inputs"),
                }
            });
            clip_summary(ui, anim, true);
        }
        NodeType::AutoTPose { set_hip_height, hip_height } => {
            group(ui, "Pose", Some("Rotations zeroed, the root at the origin, the hips centred. One frame, no animation"), |f| {
                let rest = anim.input.as_ref().and_then(|c| c.hip_joint().map(|h| (c.joints[h].name.clone(), c.joints[h].rest.translation.y * 100.0)));
                if let Some((name, h)) = &rest { f.value("Hips", format!("{name}, {h:.2} cm in the file")); }
                f.toggle("Set hip height", None, set_hip_height);
                f.row("Hip height", None, |ui| {
                    ui.add_enabled_ui(*set_hip_height, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(hip_height).speed(0.1).range(0.0..=300.0).suffix(" cm"));
                            if let Some((_, h)) = &rest { if ui.small_button("From file").clicked() { *hip_height = *h; } }
                        });
                    });
                });
            });
        }
        NodeType::FixPose { edits } => {
            let joints: Vec<String> = anim.input.as_ref().map(|c| c.joints.iter().map(|j| j.name.clone()).collect()).unwrap_or_default();
            let mut add = false;
            group_with(ui, "Corrections", Some("Added on every frame. Rotation in degrees, in the parent's space"), |ui| {
                if ui.small_button("+").on_hover_text("Add a correction").clicked() { add = true; }
            }, |f| { if edits.is_empty() { f.status(Status::Info, "None yet"); } });
            let mut remove = None;
            for (n, e) in edits.iter_mut().enumerate() {
                group_with(ui, &format!("Correction {}", n + 1), None, |ui| {
                    if ui.small_button("🗑").on_hover_text("Remove this correction").clicked() { remove = Some(n); }
                }, |f| {
                    pattern_rows(f, &format!("fix_joint{n}"), "Joints", &mut e.joint, &joints, false);
                    f.xyz("Rotate", None, &mut e.rotation, 0.5, "°");
                    f.xyz("Translate", None, &mut e.translation, 0.1, " cm");
                });
            }
            if let Some(n) = remove { edits.remove(n); }
            if add { edits.push(PoseEdit { joint: String::new(), rotation: [0.0; 3], translation: [0.0; 3] }); }
        }
        NodeType::ProxySkin { thickness } => group(ui, "Proxy", Some("A sphere per joint and a cylinder per bone, each bound to one bone, built in the pose of the first frame"), |f| {
            f.slider("Thickness", None, thickness, 0.25..=4.0, "");
            if let Some(skin) = anim.output.as_ref().and_then(|c| c.skin.as_ref()) {
                f.value("Mesh", format!("{} vertices, {} faces", skin.positions.len(), skin.faces.len()));
            }
        }),
        NodeType::Ragdoll { settings, view } => body_collide(ui, settings, view, &collide_in),

        // ── Modelling ────────────────────────────────────────────────────────
        NodeType::EditPoly { ops, pending, edit, auto_collapse } => edit_poly(ui, ops, pending, edit, auto_collapse, io),
    }
    if resync { graph.sync_sockets(sel_id); }
}

// ── Body Collide ──────────────────────────────────────────────────────────────

fn body_collide(
    ui: &mut egui::Ui, settings: &mut crate::ragdoll::Settings, view: &mut BodyView,
    inputs: &(Option<Arc<AnimData>>, Option<Arc<crate::types::MeshData>>),
) {
    let (clip, collider) = inputs;
    group(ui, "Inputs", Some("The character follows the capture and is kept out of the collider and out of itself. Joints give way, each as far as that joint may. No bone changes length"), |f| {
        match clip {
            None => f.status(Status::Info, "Connect a clip with a human skeleton to the first input"),
            Some(c) => f.value("Clip", format!("{} frames, {}", c.frames, match &c.skin {
                Some(s) => format!("skin of {} vertices", s.positions.len()), None => "no skin: capsules stand in".into() })),
        }
        match collider {
            Some(m) => f.value("Collider", format!("{} triangles", m.indices.len() / 3)),
            None => f.value("Collider", "None: the character collides with itself only"),
        }
    });
    group(ui, "Display", Some("What the viewport draws of the character. The node puts out the clip with its skin either way"), |f| {
        f.choice("Show", Some("Pieces: the skin cut into one rigid piece per body. Hulls: the shapes the solver collides"),
            view, &[(BodyView::Skin, "Skin"), (BodyView::Pieces, "Pieces"), (BodyView::Hulls, "Hulls")]);
    });
    group(ui, "Contact", None, |f| {
        f.slider("Margin", Some("Soft margin kept between the character and a surface"), &mut settings.margin, 0.0..=5.0, " cm");
        f.slider("Friction", None, &mut settings.friction, 0.0..=2.0, "");
        f.slider("Release speed", Some("How fast a limb may be pushed out of a surface"), &mut settings.release, 5.0..=300.0, " cm/s");
        f.toggle("Self collision", Some("Keep the character out of itself"), &mut settings.self_collision);
        f.row("Self overlap", Some("How far its parts may overlap"), |ui| {
            ui.add_enabled_ui(settings.self_collision, |ui| {
                form::slider_look(ui);
                ui.add(egui::Slider::new(&mut settings.self_slack, 0.0..=10.0).suffix(" cm"));
            });
        });
        f.choice("Hull detail", Some("How closely the convex hulls follow the skin. Also what Display: Hulls shows"),
            &mut settings.detail, &[(0, "Coarse"), (1, "Medium"), (2, "Fine")]);
    });
    group(ui, "Capture", Some("How closely the character follows the capture"), |f| {
        f.slider("Stiffness", None, &mut settings.stiffness, 0.25..=3.0, "");
        f.slider("Limb release", Some("A limb further than this from the capture is let go"), &mut settings.limb_limit, 5.0..=100.0, " cm");
        f.slider_u32("Smoothing", Some("Frames on each side over which the result is evened out"), &mut settings.smooth, 0..=8, "");
    });
    group(ui, "Trunk", Some("Hips and spine sunk in a seat are lifted onto it, up to Max lift. Deeper than that, they are lifted that far and left in by the rest"), |f| {
        f.slider("Max lift", None, &mut settings.ghost_depth, 1.0..=40.0, " cm");
        f.slider("Rest depth", Some("How far the trunk may rest in a surface"), &mut settings.sink, 0.0..=15.0, " cm");
    });
    group(ui, "Pass through", Some("Where the capture takes the trunk through a surface, as when an actor walks through a door that was not there on the day, the character follows the capture with collisions off, and is caught again after"), |f| {
        f.slider_u32("Fade out", Some("Frames"), &mut settings.fade_out, 1..=60, "");
        f.slider_u32("Fade in", Some("Frames"), &mut settings.fade_in, 1..=60, "");
    });
    group(ui, "Solve", None, |f| {
        let Some(c) = clip else { f.status(Status::Info, "Nothing to solve"); return };
        let key = crate::ragdoll::key(c, collider.as_ref(), settings);
        let job = crate::ragdoll::job(key).or_else(|| { crate::ragdoll::solved(key); crate::ragdoll::job(key) });
        let start = |c: &Arc<AnimData>| crate::ragdoll::start(c.clone(), collider.clone(), *settings);
        let Some(job) = job else {
            f.status(Status::Info, "Not solved for these inputs and settings: the clip passes through as it came");
            if f.buttons("", None, |ui| ui.button("Solve").clicked()) { start(c); }
            return;
        };
        let notes = job.notes.lock().unwrap().clone();
        if !notes.is_empty() { f.status(Status::Info, notes); }
        let state = job.state.lock().unwrap();
        match &*state {
            crate::ragdoll::State::Running => {
                let done = job.done.load(std::sync::atomic::Ordering::Relaxed);
                let secs = job.started.elapsed().as_secs_f32();
                let rate = done as f32 / secs.max(0.01);
                let left = if rate > 0.0 { (job.total.saturating_sub(done)) as f32 / rate } else { 0.0 };
                f.row("Progress", None, |ui| ui.add(egui::ProgressBar::new(done as f32 / job.total.max(1) as f32)
                    .text(format!("{done} of {}, {left:.0} s left", job.total))));
                drop(state);
                if f.buttons("", None, |ui| ui.button("Stop").clicked()) { crate::ragdoll::cancel(key); }
                f.ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            crate::ragdoll::State::Done(s) => {
                let r = s.report.clone();
                f.status(Status::Ok, format!("Solved {} frames in {:.1} s", s.frames, s.seconds));
                drop(state);
                f.value("In contact", format!("{} frames", r.contact_frames));
                f.value("Furthest", format!("{:.1} cm from the capture", r.max_deviation * 100.0));
                f.value("Deepest", format!("{:.1} cm left in the collider", r.max_residual * 100.0));
                if r.ghost_ranges.is_empty() {
                    f.value("Passed through", "Never: collisions were on throughout");
                } else {
                    f.value("Passed through", format!("{} frames in {} stretch{}", r.ghost_frames, r.ghost_ranges.len(), if r.ghost_ranges.len() == 1 { "" } else { "es" }));
                    f.row("", None, |ui| {
                        egui::ScrollArea::vertical().id_source("collide_ghosts").max_height(110.0).show(ui, |ui| {
                            for (a, b) in &r.ghost_ranges {
                                ui.colored_label(ink::DIM(), egui::RichText::new(format!("{} to {}  (frames {} to {})",
                                    c.timecode(c.start_frame + *a as i64), c.timecode(c.start_frame + *b as i64), a, b)).monospace().small());
                            }
                        });
                    });
                }
                f.buttons("", None, |ui| {
                    if ui.button("Solve again").clicked() { start(c); }
                    if ui.button("Forget").on_hover_text("Remove the solved result, from memory and from disk").clicked() { crate::ragdoll::clear(key); }
                });
            }
            crate::ragdoll::State::Failed(e) => {
                let e = e.clone();
                drop(state);
                f.status(Status::Bad, e);
                if f.buttons("", None, |ui| ui.button("Solve").clicked()) { start(c); }
            }
            crate::ragdoll::State::Cancelled => {
                drop(state);
                f.status(Status::Info, "Stopped: the clip passes through as it came");
                if f.buttons("", None, |ui| ui.button("Solve").clicked()) { start(c); }
            }
        }
    });
}

// ── Edit Poly ─────────────────────────────────────────────────────────────────

fn edit_poly(
    ui: &mut egui::Ui, ops: &mut Vec<PolyOp>, pending: &mut PolySelection, edit: &mut Option<usize>,
    auto_collapse: &mut bool, io: &mut PanelIo,
) {
    if edit.map(|i| i >= ops.len() || ops[i].collapsed).unwrap_or(false) { *edit = None; }
    let input = io.poly_input.clone();
    // Mesh the selection under edit applies to, and the final mesh.
    let stage = input.as_ref().map(|m| eval_cached(m, ops, edit.unwrap_or(ops.len())));
    let end   = input.as_ref().map(|m| eval_cached(m, ops, ops.len()));

    let sel_title = match *edit { Some(i) => format!("Selection of #{} {}", i + 1, ops[i].kind.label()), None => "Selection".into() };
    let mut done = false;
    group_with(ui, &sel_title, Some("The selection the next operation uses, built in the viewport or by rule"), |ui| {
        if edit.is_some() && ui.small_button("Done").on_hover_text("Back to the selection for the next operation").clicked() { done = true; }
    }, |f| {
        if edit.is_some() { f.status(Status::Info, "The viewport shows the mesh before this operation"); }
        let sel = match *edit { Some(i) if i < ops.len() => &mut ops[i].selection, _ => &mut *pending };
        selection_rows(f, sel, stage.as_deref());
        if input.is_none() { f.status(Status::Info, "No mesh connected"); }
    });
    if done { *edit = None; }

    group(ui, "Viewport tool", Some("Drag a handle in the viewport: each drag is stored as a Transform operation"), |f| {
        let mut t = *io.tool;
        f.choice("Tool", Some("Keys Q, W, E, R"), &mut t, &[(Tool::Select, "Select"), (Tool::Move, "Move"), (Tool::Rotate, "Rotate"), (Tool::Scale, "Scale")]);
        *io.tool = t;
    });

    group(ui, "Add operation", Some("Each uses the selection above"), |f| {
        let base = pending.level.base();
        let (v, e, p) = (base == SubLevel::Vertex, base == SubLevel::Edge, base == SubLevel::Polygon);
        let mut add: Option<PolyOpKind> = None;
        let mut line = |f: &mut Rows, title: &str, items: &[(&str, bool, PolyOpKind)]| {
            f.buttons(title, None, |ui| {
                for (label, on, kind) in items {
                    if ui.add_enabled(*on, egui::Button::new(*label)).clicked() { add = Some(kind.clone()); }
                }
            });
        };
        line(f, "Polygon", &[
            ("Extrude", true, PolyOpKind::Extrude { height: 0.25, mode: ExtrudeMode::Group }),
            ("Bevel",   true, PolyOpKind::Bevel { height: 0.25, outline: -0.1, mode: ExtrudeMode::Group }),
            ("Inset",   true, PolyOpKind::Inset { amount: 0.1, by_polygon: false }),
            ("Outline", true, PolyOpKind::Outline { amount: 0.05 }),
            ("Hinge",   true, PolyOpKind::Hinge { angle: 30.0, segments: 4, edge: 0 }),
            ("Bridge",  p || e, PolyOpKind::Bridge),
            ("Flip",    true, PolyOpKind::Flip),
            ("Detach",  true, PolyOpKind::Detach),
            ("Tessellate", true, PolyOpKind::Tessellate),
            ("Triangulate", true, PolyOpKind::Triangulate),
        ]);
        line(f, "Edge", &[
            ("Extrude", e, PolyOpKind::ExtrudeEdge { height: 0.0, width: 0.25 }),
            ("Chamfer", true, PolyOpKind::Chamfer { amount: 0.05 }),
            ("Connect", e || v, PolyOpKind::Connect { segments: 1 }),
            ("Remove",  e || v, PolyOpKind::Remove { clean: true }),
            ("Cap",     true, PolyOpKind::Cap),
            ("Turn",    e, PolyOpKind::Turn),
        ]);
        line(f, "Vertex", &[
            ("Extrude",  true, PolyOpKind::ExtrudeVertex { height: 0.2, width: 0.1 }),
            ("Weld",     true, PolyOpKind::Weld { threshold: 0.01 }),
            ("Collapse", true, PolyOpKind::Collapse),
            ("Break",    true, PolyOpKind::Break),
        ]);
        line(f, "Any", &[
            ("Transform",   true, PolyOpKind::identity_transform()),
            ("Delete",      true, PolyOpKind::Delete),
            ("Make planar", true, PolyOpKind::MakePlanar { axis: None }),
            ("Relax",       true, PolyOpKind::Relax { amount: 0.5, iterations: 1, hold_border: true }),
            ("Slice",       true, PolyOpKind::Slice { axis: 1, offset: 0.0 }),
            ("Insert vertex", true, PolyOpKind::InsertVertex { segments: 1 }),
        ]);
        line(f, "Whole mesh", &[
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

    let live = ops.iter().filter(|op| !op.collapsed).count();
    let mut collapse_everything = false;
    group_with(ui, "Operations", None, |ui| {
        if ui.add_enabled(live > 0, egui::Button::new("Collapse all").small()).clicked() { collapse_everything = true; }
    }, |f| {
        f.choice("Mode", Some("Keep live: operations stay editable until collapsed. Auto-collapse: adding one collapses those before it"),
            auto_collapse, &[(false, "Keep live"), (true, "Auto-collapse")]);
        f.value("Count", format!("{} live, {} collapsed", live, ops.len() - live));
        if let Some(m) = &end { f.value("Result", format!("{} vertices, {} polygons", m.verts.len(), m.polys.len())); }
    });
    if collapse_everything { collapse_all(ops); *edit = None; }

    enum ListEdit { Up(usize), Down(usize), Delete(usize), Collapse(usize), Restore(usize) }
    let mut change = None;
    let count = ops.len();
    let mut i = 0;
    while i < count {
        if ops[i].collapsed {
            let mut j = i;
            while j + 1 < count && ops[j + 1].collapsed { j += 1; }
            let n = j - i + 1;
            let title = if n == 1 { format!("#{} {}  (collapsed)", i + 1, ops[i].kind.label()) } else { format!("#{} to #{}  ({n} collapsed)", i + 1, j + 1) };
            let names = ops[i..=j].iter().map(|o| o.kind.label()).collect::<Vec<_>>().join(", ");
            group_with(ui, &title, Some(&names), |ui| {
                if ui.small_button("Restore").on_hover_text("Make editable again").clicked() { change = Some(ListEdit::Restore(i)); }
            }, |_| {});
            i = j + 1;
            continue;
        }
        let title = format!("#{} {}", i + 1, ops[i].kind.label());
        let op = &mut ops[i];
        group_with(ui, &title, None, |ui| {
            if ui.small_button("🗑").on_hover_text("Remove").clicked() { change = Some(ListEdit::Delete(i)); }
            if ui.small_button("✔").on_hover_text("Collapse").clicked() { change = Some(ListEdit::Collapse(i)); }
            if ui.add_enabled(i + 1 < count, egui::Button::new("⏷").small()).on_hover_text("Later").clicked() { change = Some(ListEdit::Down(i)); }
            if ui.add_enabled(i > 0, egui::Button::new("⏶").small()).on_hover_text("Earlier").clicked() { change = Some(ListEdit::Up(i)); }
            if ui.selectable_label(*edit == Some(i), "🎯").on_hover_text("Edit this operation's selection").clicked() {
                *edit = if *edit == Some(i) { None } else { Some(i) };
            }
            form::check(ui, &mut op.enabled).on_hover_text("On");
        }, |f| op_rows(f, i, &mut op.kind));
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
}

/// The parameters of one Edit Poly operation.
fn op_rows(f: &mut Rows, i: usize, kind: &mut PolyOpKind) {
    let modes = [(ExtrudeMode::Group, "Group"), (ExtrudeMode::LocalNormal, "Local normal"), (ExtrudeMode::ByPolygon, "By polygon")];
    let mut mode_row = |f: &mut Rows, mode: &mut ExtrudeMode| {
        let id = f.ui.id().with(("poly_mode", i));
        f.row("Mode", None, |ui| form::choice_widget(ui, id, mode, &modes));
    };
    match kind {
        PolyOpKind::Extrude { height, mode } => { f.drag("Height", None, height, 0.005, ""); mode_row(f, mode); }
        PolyOpKind::Bevel { height, outline, mode } => {
            f.drag("Height", None, height, 0.005, "");
            f.drag("Outline", None, outline, 0.005, "");
            mode_row(f, mode);
        }
        PolyOpKind::Inset { amount, by_polygon } => { f.drag("Amount", None, amount, 0.005, ""); f.toggle("By polygon", None, by_polygon); }
        PolyOpKind::Transform { translate, rotate, scale, falloff } => {
            f.xyz("Translate", None, translate, 0.01, "");
            // Shown and edited as XYZ angles in degrees.
            let q = Quat::from_array(*rotate).normalize();
            let (x, y, z) = q.to_euler(EulerRot::XYZ);
            let mut deg = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
            if f.xyz("Rotate", None, &mut deg, 0.5, "°") {
                *rotate = Quat::from_euler(EulerRot::XYZ, deg[0].to_radians(), deg[1].to_radians(), deg[2].to_radians()).to_array();
            }
            f.xyz("Scale", None, scale, 0.01, "");
            f.drag("Soft falloff", Some("Soft selection: vertices within this distance of the selection follow part of the way. 0 is off"), falloff, 0.01, "");
            *falloff = falloff.max(0.0);
        }
        PolyOpKind::Chamfer { amount } | PolyOpKind::Outline { amount } => { f.drag("Amount", None, amount, 0.005, ""); }
        PolyOpKind::ExtrudeVertex { height, width } | PolyOpKind::ExtrudeEdge { height, width } => {
            f.drag("Height", None, height, 0.005, "");
            f.drag("Width", None, width, 0.005, "");
        }
        PolyOpKind::Hinge { angle, segments, edge } => {
            f.drag("Angle", None, angle, 0.5, "°");
            *angle = angle.clamp(-360.0, 360.0);
            f.drag_u32("Segments", None, segments, 1..=64);
            f.drag_u32("Hinge edge", Some("Which edge of the selection's outline is the hinge"), edge, 0..=9999);
        }
        PolyOpKind::Slice { axis, offset } => {
            f.choice("Across", None, axis, &[(0, "X"), (1, "Y"), (2, "Z")]);
            f.drag("At", None, offset, 0.005, "");
        }
        PolyOpKind::InsertVertex { segments } => { f.drag_u32("Per edge", None, segments, 1..=64); }
        PolyOpKind::Remove { clean } => { f.toggle("Clean up", Some("Also remove the vertices left over"), clean); }
        PolyOpKind::Weld { threshold } => { f.drag("Threshold", None, threshold, 0.001, ""); }
        PolyOpKind::Connect { segments } => { f.drag_u32("Segments", None, segments, 1..=64); }
        PolyOpKind::MakePlanar { axis } => { f.choice("Plane", None, axis, &[(None, "Best fit"), (Some(0), "X"), (Some(1), "Y"), (Some(2), "Z")]); }
        PolyOpKind::Relax { amount, iterations, hold_border } => {
            f.drag("Amount", None, amount, 0.005, "");
            f.drag_u32("Iterations", None, iterations, 1..=200);
            f.toggle("Hold borders", Some("Open borders stay where they are"), hold_border);
        }
        PolyOpKind::Subdivide { iterations } => { f.drag_u32("Iterations", None, iterations, 1..=4); }
        PolyOpKind::Delete | PolyOpKind::Collapse | PolyOpKind::Cap | PolyOpKind::Bridge
        | PolyOpKind::Detach | PolyOpKind::Break | PolyOpKind::Flip | PolyOpKind::Tessellate
        | PolyOpKind::Triangulate | PolyOpKind::Turn => {}
    }
}

/// Sub-object level, where the selection comes from, and its modifiers.
fn selection_rows(f: &mut Rows, sel: &mut PolySelection, mesh: Option<&PolyMesh>) {
    f.choice("Level", None, &mut sel.level, &[
        (SubLevel::Vertex, "Vertex"), (SubLevel::Edge, "Edge"), (SubLevel::Border, "Border"),
        (SubLevel::Polygon, "Polygon"), (SubLevel::Element, "Element"),
    ]);
    #[derive(Clone, Copy, PartialEq)]
    enum Src { Picked, All, Normal, Box }
    let mut src = match sel.source { SelSource::Picked => Src::Picked, SelSource::All => Src::All, SelSource::ByNormal { .. } => Src::Normal, SelSource::InBox { .. } => Src::Box };
    if f.choice("Source", Some("Picked: click or drag a box in the viewport, Ctrl adds, Shift removes"),
        &mut src, &[(Src::Picked, "Picked"), (Src::All, "All"), (Src::Normal, "By normal"), (Src::Box, "In box")]) {
        sel.source = match src {
            Src::Picked => SelSource::Picked,
            Src::All => SelSource::All,
            Src::Normal => SelSource::ByNormal { dir: [0.0, 1.0, 0.0], angle: 45.0 },
            Src::Box => {
                // Start with a box around the whole mesh.
                let (mut lo, mut hi) = ([-1.0f32; 3], [1.0f32; 3]);
                if let Some(m) = mesh.filter(|m| !m.verts.is_empty()) {
                    lo = [f32::MAX; 3]; hi = [f32::MIN; 3];
                    for v in &m.verts { for a in 0..3 { lo[a] = lo[a].min(v[a]); hi[a] = hi[a].max(v[a]); } }
                }
                SelSource::InBox { min: lo, max: hi }
            }
        };
    }
    match &mut sel.source {
        SelSource::ByNormal { dir, angle } => {
            f.choice("Facing", None, dir, &[([1.0, 0.0, 0.0], "+X"), ([-1.0, 0.0, 0.0], "-X"), ([0.0, 1.0, 0.0], "+Y"),
                ([0.0, -1.0, 0.0], "-Y"), ([0.0, 0.0, 1.0], "+Z"), ([0.0, 0.0, -1.0], "-Z")]);
            f.drag("Within", None, angle, 0.5, "°");
            *angle = angle.clamp(0.0, 180.0);
        }
        SelSource::InBox { min, max } => {
            f.xyz("Box min", None, min, 0.01, "");
            f.xyz("Box max", None, max, 0.01, "");
        }
        SelSource::Picked | SelSource::All => {}
    }
    f.buttons("Modify", None, |ui| {
        if ui.button("Shrink").clicked() { sel.grow -= 1; }
        if ui.button("Grow").clicked() { sel.grow += 1; }
        if sel.grow != 0 { ui.colored_label(ink::DIM(), format!("{:+}", sel.grow)); }
        if ui.button("Clear").clicked() { let level = sel.level; *sel = PolySelection { level, ..Default::default() }; }
    });
    f.toggle("Invert", None, &mut sel.invert);
    if let (Some(m), SubLevel::Edge) = (mesh, sel.level) {
        f.buttons("Extend", None, |ui| {
            if ui.button("Loop").on_hover_text("Extend the selected edges along their loops").clicked() { sel.expand_edges(m, false); }
            if ui.button("Ring").on_hover_text("Extend the selected edges across their rings").clicked() { sel.expand_edges(m, true); }
        });
    }
    if let Some(m) = mesh {
        let what = match sel.level.base() { SubLevel::Vertex => "vertices", SubLevel::Polygon => "polygons", _ => "edges" };
        let polys = sel.poly_mask(m).iter().filter(|s| **s).count();
        let text = if sel.level.base() == SubLevel::Polygon { format!("{polys} polygons") }
            else { format!("{} {what}, on {polys} polygons", sel.count(m)) };
        f.value("Selected", text);
    }
}

// ── Shared rows ───────────────────────────────────────────────────────────────

/// Path field with a button that opens the file browser.
fn path_row(f: &mut Rows, value: &mut String, hint: &str, io: &mut PanelIo, node: NodeId, mode: BrowseMode, title: &str, exts: &[&str]) {
    f.row("Path", None, |ui| {
        ui.horizontal(|ui| {
            let w = ui.available_width() - 34.0;
            ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(w.max(40.0)));
            if ui.button("📂").on_hover_text("Browse").clicked() { io.browser.open(BrowseTarget::Node(node), mode, title, exts, ""); }
        });
    });
}

/// Loaded or not, and the take when the file has several.
fn take_rows(f: &mut Rows, loaded: Result<crate::fbx_loader::LoadedFbx, String>, take: &mut u32) {
    match loaded {
        Ok(loaded) => {
            f.status(Status::Ok, "Loaded");
            if loaded.takes.len() > 1 {
                let options: Vec<(u32, &str)> = loaded.takes.iter().enumerate().map(|(i, n)| (i as u32, n.as_str())).collect();
                f.choice("Take", None, take, &options);
            }
        }
        Err(e) => f.status(Status::Bad, e),
    }
}

fn rate_row(f: &mut Rows, num: &mut u32, den: &mut u32) {
    let mut cur = FrameRate::new(*num, *den);
    let labels: Vec<String> = RATE_PRESETS.iter().map(|(name, _)| format!("{name} fps")).collect();
    let options: Vec<(FrameRate, &str)> = RATE_PRESETS.iter().zip(&labels).map(|((_, r), l)| (*r, l.as_str())).collect();
    if f.choice("Rate", None, &mut cur, &options) { *num = cur.num; *den = cur.den; }
}

/// What comes in and what goes out of a clip node.
fn clip_summary(ui: &mut egui::Ui, anim: &AnimContext, with_input: bool) {
    let line = |c: &AnimData| format!("{} to {}, {} frames at {} fps{}",
        c.timecode(c.start_frame), c.timecode(c.end_frame()), c.frames, c.rate.label(), if c.drop_frame { " DF" } else { "" });
    group(ui, "Clip", None, |f| {
        if with_input {
            match &anim.input { Some(i) => f.value("In", line(i)), None => f.status(Status::Info, "No clip connected") }
        }
        if let Some(o) = &anim.output { f.value("Out", line(o)); }
    });
}

/// Progress and results of the last write.
fn batch_log(ui: &mut egui::Ui, batch: &BatchState) {
    let p = batch.0.lock().unwrap();
    if p.log.is_empty() && !p.running { return; }
    group(ui, "Log", None, |f| f.wide(|ui| {
        if p.running {
            ui.add(egui::ProgressBar::new(p.done as f32 / p.total.max(1) as f32).text(format!("Writing {} of {}", p.done, p.total)));
        }
        egui::ScrollArea::vertical().id_source("batch_log").max_height(260.0).stick_to_bottom(true).show(ui, |ui| {
            for line in &p.log {
                let col = if line.starts_with("FAILED") { ink::BAD() } else { ink::LABEL() };
                ui.colored_label(col, egui::RichText::new(line).small());
            }
        });
    }));
}

/// What a USD file holds, from the cached stage.
fn usd_summary(ui: &mut egui::Ui, path: &str) {
    let scene = match crate::usd_scene::load_cached(path) {
        Ok(s) => s,
        Err(e) => { group(ui, "Stage", None, |f| f.status(Status::Bad, format!("Could not read the file: {e}"))); return; }
    };
    let leaf = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    group(ui, "Stage", None, |f| {
        f.value("Meshes", format!("{} primitives, {} triangles", scene.meshes.len(), scene.triangles()));
        f.value("Up axis", scene.up_axis.to_string());
        f.value("Unit", match scene.meters_per_unit { Some(u) => format!("{u} m per unit, read as metres"), None => "Not stated, left as it is".into() });
        if let Some((start, end, rate)) = scene.time { f.value("Time", format!("{start} to {end} at {rate} per second")); }
        let kinds: Vec<String> = scene.prim_counts.iter().map(|(k, n)| format!("{n} {k}")).collect();
        f.value("Prims", kinds.join(", "));
    });
    let list = |ui: &mut egui::Ui, title: String, rows: Vec<(String, String)>| {
        if rows.is_empty() { return; }
        egui::CollapsingHeader::new(title).default_open(false).show(ui, |ui| {
            egui::ScrollArea::vertical().id_source(ui.id().with("usd_rows")).max_height(180.0).show(ui, |ui| {
                for (name, detail) in rows {
                    ui.colored_label(ink::LABEL(), name);
                    if !detail.is_empty() { ui.colored_label(ink::DIM(), egui::RichText::new(detail).small()); }
                }
            });
        });
    };
    list(ui, format!("Materials ({})", scene.materials.len()), scene.materials.iter().map(|m| {
        let mut parts = vec![];
        if let Some(c) = m.diffuse { parts.push(format!("colour {:.2} {:.2} {:.2}", c[0], c[1], c[2])); }
        if let Some(v) = m.roughness { parts.push(format!("roughness {v:.2}")); }
        if let Some(v) = m.metallic { parts.push(format!("metallic {v:.2}")); }
        if let Some(v) = m.opacity { parts.push(format!("opacity {v:.2}")); }
        for (role, file) in &m.textures { parts.push(format!("{role}: {file}")); }
        (leaf(&m.path), parts.join("\n"))
    }).collect());
    list(ui, format!("Cameras ({})", scene.cameras.len()), scene.cameras.iter().map(|c| {
        (leaf(&c.path), format!("{} {} mm, aperture {} x {}, clip {} to {}, at {:.2} {:.2} {:.2}",
            c.projection, c.focal_length, c.aperture[0], c.aperture[1], c.clip[0], c.clip[1], c.position[0], c.position[1], c.position[2]))
    }).collect());
    list(ui, format!("Skeletons ({})", scene.skeletons.len()), scene.skeletons.iter().map(|s| (leaf(&s.path), format!("{} joints", s.joints.len()))).collect());
    list(ui, format!("Lights ({})", scene.lights.len()), scene.lights.iter().map(|(p, k)| (leaf(p), k.clone())).collect());
    if !scene.notes.is_empty() {
        ui.add_space(form::GAP);
        group(ui, "Not read in full", None, |f| { for n in &scene.notes { f.status(Status::Info, n.clone()); } });
    }
}

/// A name pattern: comma-separated regular expressions (see
/// `core::pattern`), with a picker of the names coming in and a match count.
/// With `single`, the field holds one expression and picking replaces it.
fn pattern_rows(f: &mut Rows, id: &str, label: &str, text: &mut String, names: &[String], single: bool) {
    use crate::core::pattern;
    let hint = if single { "A regular expression, case ignored" }
        else { "Comma-separated regular expressions, case ignored. A plain word matches names containing it" };
    f.row(label, Some(hint), |ui| {
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 48.0).max(60.0);
            ui.add(egui::TextEdit::singleline(text).desired_width(w).hint_text("regex, regex"));
            ui.add_enabled_ui(!names.is_empty(), |ui| {
                ui.menu_button("Pick", |ui| {
                    let filter_id = ui.id().with(id).with("filter");
                    let mut filter: String = ui.data(|d| d.get_temp(filter_id)).unwrap_or_default();
                    ui.add(egui::TextEdit::singleline(&mut filter).desired_width(200.0).hint_text("filter the list"));
                    let shown = pattern::NamePattern::new(&filter);
                    let current = pattern::NamePattern::new(text);
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for n in names {
                            if !shown.is_empty() && !shown.matches(n) { continue; }
                            if single {
                                if ui.selectable_label(false, n).clicked() { *text = regex::escape(n); ui.close_menu(); }
                            } else if ui.selectable_label(current.matches(n), n).clicked() {
                                pattern::toggle(text, n);
                            }
                        }
                    });
                    if !single && !filter.trim().is_empty() && ui.button("Use the filter as a pattern").clicked() {
                        let mut list: Vec<String> = pattern::parts(text).into_iter().map(String::from).collect();
                        list.extend(pattern::parts(&filter).into_iter().map(String::from));
                        *text = list.join(", ");
                        filter.clear();
                    }
                    ui.data_mut(|d| d.insert_temp(filter_id, filter));
                }).response.on_hover_text("Pick from the names coming in");
            });
        });
    });
    if let Some(e) = pattern::error(text) { f.status(Status::Bad, e); }
    else if !names.is_empty() && !text.trim().is_empty() {
        let p = pattern::NamePattern::new(text);
        let n = names.iter().filter(|x| p.matches(x)).count();
        f.status(Status::Info, format!("Matches {n} of {}", names.len()));
    }
}

/// The axes and unit a clip is written in.
fn space_line(c: &AnimData) -> String {
    let axis = |(a, sgn): (i32, i32)| format!("{}{}", if sgn < 0 { "-" } else { "" }, ["X", "Y", "Z"][a.clamp(0, 2) as usize]);
    let unit = |cm: f64| match cm { x if (x - 1.0).abs() < 1e-9 => "cm".to_string(), x if (x - 100.0).abs() < 1e-9 => "m".into(),
        x if (x - 0.1).abs() < 1e-9 => "mm".into(), x if (x - 2.54).abs() < 1e-9 => "inches".into(), x => format!("{x} cm units") };
    match c.space.as_deref() {
        Some(sp) => format!("{} up, {}, as the source file", axis(sp.up), unit(sp.unit_cm)),
        None => "Y up, cm (made in this program)".into(),
    }
}
