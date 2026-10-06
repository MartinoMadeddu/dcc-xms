mod core;
mod types;
mod node_graph;
mod scene_graph;
mod properties;
mod viewport;
mod ice;
mod usd_loader;
mod prim_inspector;
mod fbx_loader;
mod fbx_writer;
mod file_browser;
mod batch;
mod graph_io;
mod timeline;
mod theme;
mod modelling;
mod layout;

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin};

use types::{MainCamera, GeneratedMesh, GroundGrid, MeshData, SceneHierarchy, SubnetId, ViewportRect, PrimInspectorState};
use node_graph::NodeGraphState;
use scene_graph::OperatorStack;
use viewport::camera::{CameraOrbitState, camera_controller, focus_camera, draw_origin_label};
use ice::{SubnetStore, GraphNavigation, ui::{draw_subnet_graph, draw_breadcrumb}};
use node_graph::ui::draw_node_graph;
use scene_graph::ui::{draw_scene_explorer, draw_operator_stack};
use properties::ui::draw_properties_panel;
use prim_inspector::ui::draw_prim_inspector;
use types::NodeType;
use timeline::{Playback, TimelineState, resolve_source, ui::draw_timeline};
use properties::ui::{AnimContext, PanelAction, PanelIo};
use file_browser::{BrowseMode, BrowseTarget, FileBrowser};
use batch::BatchState;
use layout::{Layout, Pane};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(EguiPlugin)
        .init_resource::<NodeGraphState>()
        .init_resource::<CameraOrbitState>()
        .init_resource::<OperatorStack>()
        .init_resource::<SceneHierarchy>()
        .init_resource::<SubnetStore>()
        .init_resource::<GraphNavigation>()
        .init_resource::<ViewportRect>()
        .init_resource::<PrimInspectorState>()
        .init_resource::<Playback>()
        .init_gizmo_group::<modelling::PolyGizmos>()
        .init_resource::<FileBrowser>()
        .init_resource::<BatchState>()
        .init_resource::<GraphFile>()
        .init_resource::<modelling::PolyTool>()
        .init_resource::<Layout>()
        .init_resource::<node_graph::GraphRevision>()
        .init_resource::<viewport::nav::NavSettings>()
        .add_systems(Startup, (setup_scene, setup_egui_theme, setup_gizmos, modelling::setup_gizmos))
        .add_systems(Update, (
            dcc_ui,
            track_revision.after(dcc_ui).after(modelling::pick_system),
            update_operator_stack.after(track_revision),
            update_scene_hierarchy.after(track_revision),
            update_generated_meshes.after(track_revision),
            apply_viewport_rect.after(dcc_ui),
            camera_controller,
            focus_camera,
            draw_origin_label,
            draw_skeleton.after(dcc_ui),
            modelling::pick_system.after(dcc_ui),
            modelling::overlay_system.after(dcc_ui),
        ))
        .run();
}
// -- Egui theme setup ------------------------------------------------

fn setup_egui_theme(mut contexts: EguiContexts) {
    theme::init(contexts.ctx_mut());
}

// ── UI ────────────────────────────────────────────────────────────────────────

fn dcc_ui(
    mut contexts:   EguiContexts,
    mut graph:      ResMut<NodeGraphState>,
    mut stack:      ResMut<OperatorStack>,
    mut hierarchy:  ResMut<SceneHierarchy>,
    mut subnets:    ResMut<SubnetStore>,
    mut nav:        ResMut<GraphNavigation>,
    mut vp_rect:    ResMut<ViewportRect>,
    mut prim_state: ResMut<PrimInspectorState>,
    mut playback:   ResMut<Playback>,
    mut browser:    ResMut<FileBrowser>,
    mut graph_file: ResMut<GraphFile>,
    mut poly_tool:  ResMut<modelling::PolyTool>,
    mut nav_settings: ResMut<viewport::nav::NavSettings>,
    (mut layout, revision): (ResMut<Layout>, Res<node_graph::GraphRevision>),
    batch:          Res<BatchState>,
    time:           Res<Time>,
) {
    let ctx = contexts.ctx_mut();

    // ── Path chosen in the file browser ──────────────────────────────────────
    if let Some((target, path)) = browser.take_result() {
        let text = path.to_string_lossy().to_string();
        match target {
            BrowseTarget::Node(id) => {
                if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == id) {
                    match &mut node.node_type {
                        NodeType::LoadUsd { path } | NodeType::LoadFbx { path, .. } => *path = text,
                        NodeType::LoadFbxDir { dir, index, .. } => { *dir = text; *index = 0; }
                        // A folder was picked: keep the file name pattern.
                        NodeType::WriteFbx { path } => {
                            let name = path.rsplit(['/', '\\']).next().filter(|n| !n.is_empty())
                                .unwrap_or("{file}_{char}.fbx").to_string();
                            *path = std::path::Path::new(&text).join(name).to_string_lossy().to_string();
                        }
                        _ => {}
                    }
                }
            }
            BrowseTarget::OpenGraph => {
                graph_file.message = match graph_io::load(&mut graph, &path) {
                    Ok(())  => format!("Opened {}", path.display()),
                    Err(e)  => format!("Could not open {}: {e}", path.display()),
                };
            }
            BrowseTarget::SaveGraph => {
                graph_file.message = match graph_io::save(&graph, &path) {
                    Ok(())  => format!("Saved {}", path.display()),
                    Err(e)  => format!("Could not save {}: {e}", path.display()),
                };
            }
        }
    }

    // Range, rate and timecode come from the clip of the selected node.
    let timeline_state = resolve_source(&graph);
    let keys_free      = !ctx.wants_keyboard_input() && !browser.is_open();

    // Clips around the selected node, for the properties panel.
    let selected_input = graph.selected_node
        .and_then(|id| graph.nodes.iter().find(|n| n.id == id))
        .and_then(|n| n.inputs.first())
        .and_then(|i| i.connected_output)
        .and_then(|(src, out)| graph.eval_anim_out(src, out));
    let anim_ctx = AnimContext {
        time:   playback.time,
        output: match &timeline_state {
            TimelineState::Source(s) if s.from_selection => Some(s.clip.clone()),
            _ => None,
        },
        input:  selected_input,
    };

    // ── Panes ────────────────────────────────────────────────────────────────
    // Every pane is a tab of one dock area: drag a tab to move it, stack it
    // with another, or pull it out as a window.
    vp_rect.0 = None;   // set again by the viewport pane if it is showing
    let mut reset_layout = false;
    {
        let mut panes = Panes {
            graph: &mut graph, stack: &mut stack, hierarchy: &mut hierarchy, subnets: &mut subnets,
            nav: &mut nav, vp_rect: &mut vp_rect, prim_state: &mut prim_state, playback: &mut playback,
            browser: &mut browser, graph_file: &mut graph_file, poly_tool: &mut poly_tool,
            batch: &batch, anim_ctx: &anim_ctx, timeline_state: &timeline_state,
            dt: time.delta_seconds_f64(), keys_free,
            space_plays: nav_settings.style != viewport::nav::NavStyle::Houdini,
            revision: revision.0, reset_layout: &mut reset_layout,
        };
        let mut style = egui_dock::Style::from_egui(ctx.style().as_ref());
        style.tab_bar.fill_tab_bar = true;
        egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
            egui_dock::DockArea::new(&mut layout.dock)
                .style(style)
                .show_close_buttons(false)
                .show_inside(ui, &mut panes);
        });
    }
    if reset_layout { layout.reset(); }
    // Not while a button is down: a drag changes the layout on every frame.
    if !ctx.input(|i| i.pointer.any_down()) { layout.save_if_changed(); }

    // ── File browser (on top of everything) ───────────────────────────────────
    browser.show(ctx);

    // ── Viewport navigation menu and help ─────────────────────────────────────
    let Some(vp) = vp_rect.0 else { return };
    let before = *nav_settings;
    let mut chosen = before;
    // The key list is hidden until the question mark is clicked.
    let help_id = egui::Id::new("viewport_nav_help");
    let mut nav_help = ctx.data(|d| d.get_temp::<bool>(help_id)).unwrap_or(false);
    egui::Area::new("viewport_nav_menu".into())
        .fixed_pos(vp.min + egui::vec2(10.0, 8.0))
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(theme::c(90, 90, 90).gamma_multiply(0.92))
                .stroke(egui::Stroke::new(1.0_f32, theme::c(70, 70, 70)))
                .rounding(4.0)
                .inner_margin(3.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.menu_button(format!("Navigation: {} ⏷", chosen.style.label()), |ui| {
                            for style in viewport::nav::NavStyle::ALL {
                                if ui.radio_value(&mut chosen.style, style, style.label()).clicked() { ui.close_menu(); }
                            }
                            ui.separator();
                            ui.checkbox(&mut chosen.invert_zoom, "Invert zoom drag");
                        });
                        if ui.selectable_label(nav_help, "?").on_hover_text("Show the navigation keys").clicked() {
                            nav_help = !nav_help;
                        }
                    });
                });
        });
    if chosen != before {
        *nav_settings = chosen;
        chosen.save();
    }

    ctx.data_mut(|d| d.insert_temp(help_id, nav_help));
    if nav_help {
    egui::Area::new("viewport_label".into())
        .fixed_pos(vp.min + egui::vec2(10.0, 40.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(theme::c(90, 90, 90).gamma_multiply(0.86))
                .stroke(egui::Stroke::new(1.0_f32, theme::c(70, 70, 70)))
                .rounding(6.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    let play = if chosen.style == viewport::nav::NavStyle::Houdini { "Tap Space: play  |  Left/Right: step" } else { "Space: play  |  Left/Right: step" };
                    let nav_lines = chosen.style.help();
                    for line in nav_lines.iter().chain(["Scroll: zoom", "F: focus", play].iter()) {
                        ui.label(egui::RichText::new(*line)
                            .color(egui::Color32::from_rgb(190, 190, 190)));
                    }
                });
        });
    }
}

/// Everything the panes draw from, borrowed for one frame.
struct Panes<'a> {
    graph:          &'a mut NodeGraphState,
    stack:          &'a mut OperatorStack,
    hierarchy:      &'a mut SceneHierarchy,
    subnets:        &'a mut SubnetStore,
    nav:            &'a mut GraphNavigation,
    vp_rect:        &'a mut ViewportRect,
    prim_state:     &'a mut PrimInspectorState,
    playback:       &'a mut Playback,
    browser:        &'a mut FileBrowser,
    graph_file:     &'a mut GraphFile,
    poly_tool:      &'a mut modelling::PolyTool,
    batch:          &'a BatchState,
    anim_ctx:       &'a AnimContext,
    timeline_state: &'a TimelineState,
    dt:             f64,
    keys_free:      bool,
    space_plays:    bool,
    revision:       u64,
    reset_layout:   &'a mut bool,
}

impl egui_dock::TabViewer for Panes<'_> {
    type Tab = Pane;

    fn title(&mut self, tab: &mut Pane) -> egui::WidgetText { tab.title().into() }
    fn closeable(&mut self, _tab: &mut Pane) -> bool { false }
    // Panes scroll their own content where they need to.
    fn scroll_bars(&self, _tab: &Pane) -> [bool; 2] { [false, false] }
    // The 3D view is drawn by the camera behind the interface.
    fn clear_background(&self, tab: &Pane) -> bool { *tab != Pane::Viewport }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Pane) {
        match tab {
            Pane::Viewport => {
                self.vp_rect.0 = Some(ui.max_rect());
            }

            Pane::Timeline => {
                draw_timeline(ui, self.playback, self.timeline_state, self.dt, self.keys_free, self.space_plays);
            }

            Pane::Properties => {
                // A scroll area that never shrinks keeps the pane at the size
                // it was given: text wraps, anything still too big scrolls.
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    let poly_input = self.graph.selected_node.and_then(|id| {
                        let subnets = &*self.subnets;
                        let eval = |sid: SubnetId, mesh: &MeshData, template: Option<&MeshData>| -> MeshData {
                            subnets.get(sid).map(|sg| sg.evaluate(mesh, template)).unwrap_or_else(|| mesh.clone())
                        };
                        let is_edit_poly = self.graph.nodes.iter()
                            .any(|n| n.id == id && matches!(n.node_type, NodeType::EditPoly { .. }));
                        if is_edit_poly { modelling::input_mesh(self.graph, id, &eval) } else { None }
                    });
                    let mut tool = self.poly_tool.tool;
                    let mut io = PanelIo { browser: self.browser, batch: self.batch, action: None, poly_input, tool: &mut tool };
                    draw_properties_panel(ui, self.graph, &*self.stack, self.subnets, self.nav, self.anim_ctx, &mut io);
                    if let Some(PanelAction::Write { targets, all_files }) = io.action {
                        batch::start(self.batch, self.graph, targets, all_files);
                    }
                    if tool != self.poly_tool.tool { self.poly_tool.tool = tool; }
                });
            }

            Pane::SceneExplorer => {
                egui::ScrollArea::both().id_source("scene_explorer_scroll").auto_shrink([false, false])
                    .show(ui, |ui| draw_scene_explorer(ui, self.hierarchy, self.graph));
            }

            Pane::OperatorStack => {
                egui::ScrollArea::both().id_source("operator_stack_scroll").auto_shrink([false, false])
                    .show(ui, |ui| draw_operator_stack(ui, self.stack, self.graph));
            }

            Pane::PrimInspector => {
                let get_mesh = |g: &NodeGraphState| -> Option<MeshData> {
                    let id = g.selected_node?;
                    let mut cache = std::collections::HashMap::new();
                    let eval_subnet = |_sid: SubnetId, mesh: &MeshData, _template: Option<&MeshData>| mesh.clone();
                    g.eval_node(id, &mut cache, &eval_subnet).map(|r| r.into_mesh())
                };
                draw_prim_inspector(ui, self.graph, self.prim_state, self.revision, &get_mesh);
            }

            Pane::NodeGraph => match self.nav.current_subnet {
                Some(sid) => {
                    if let Some(sg) = self.subnets.get_mut(sid) {
                        let subnet_name = sg.name.clone();
                        if draw_breadcrumb(ui, &subnet_name) {
                            self.nav.current_subnet = None;
                        } else {
                            ui.label("Right-click: add  |  Shift+drag: pan  |  Esc: cancel wire");
                            ui.separator();
                            // Canvas in its own child Ui: see the note below.
                            let rect = ui.available_rect_before_wrap();
                            let mut canvas = ui.child_ui(rect, *ui.layout(), None);
                            canvas.set_clip_rect(rect.intersect(ui.clip_rect()));
                            draw_subnet_graph(&mut canvas, sg);
                            ui.allocate_rect(rect, egui::Sense::hover());
                        }
                    } else {
                        self.nav.current_subnet = None;
                    }
                }
                None => {
                    // Wraps when the pane is narrow.
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("📂 Open").on_hover_text("Load a saved graph").clicked() {
                            self.browser.open(BrowseTarget::OpenGraph, BrowseMode::File, "Open graph", &["json"], "");
                        }
                        if ui.button("💾 Save").on_hover_text("Save this graph").clicked() {
                            self.browser.open(BrowseTarget::SaveGraph, BrowseMode::Save, "Save graph", &["json"], "graph.json");
                        }
                        let label = if theme::is_dark() { "Light mode" } else { "Dark mode" };
                        if ui.button(label).on_hover_text("Switch colour theme").clicked() {
                            theme::set_dark(ui.ctx(), !theme::is_dark());
                        }
                        if ui.button("Reset layout").on_hover_text("Put every pane back where it started").clicked() {
                            *self.reset_layout = true;
                        }
                        // Ready-made graphs. Picking one replaces the current graph.
                        ui.menu_button("Templates", |ui| {
                            if ui.button("Mocap split")
                                .on_hover_text("Folder of takes, split per character, animation and skinned T-pose written per character")
                                .clicked()
                            {
                                graph_io::mocap_split_template(self.graph);
                                self.graph_file.message = "Template loaded. Select the Takes node and choose a folder.".into();
                                ui.close_menu();
                            }
                        });
                    });
                    if !self.graph_file.message.is_empty() {
                        ui.label(egui::RichText::new(&self.graph_file.message).small());
                    }
                    ui.label("Right-click/Tab: add  |  Shift+drag: pan  |  Esc: cancel wire  |  Double-click subnet: dive in");
                    ui.separator();

                    // The canvas gets its own child Ui. Nodes are widgets placed
                    // at arbitrary positions; drawn straight into the pane, a
                    // node dragged or panned past the edge would stretch it.
                    let rect = ui.available_rect_before_wrap();
                    let mut canvas = ui.child_ui(rect, *ui.layout(), None);
                    canvas.set_clip_rect(rect.intersect(ui.clip_rect()));
                    let dive = draw_node_graph(&mut canvas, self.graph);
                    ui.allocate_rect(rect, egui::Sense::hover());

                    for node in self.graph.nodes.iter_mut() {
                        if let NodeType::Subnet { id, name } = &mut node.node_type {
                            if *id == SubnetId(usize::MAX) {
                                let new_id = self.subnets.create_subnet(name.clone());
                                *id = new_id;
                            }
                        }
                    }
                    if let Some(sid) = dive {
                        if sid != SubnetId(usize::MAX) {
                            self.nav.current_subnet = Some(sid);
                        }
                    }
                }
            },
        }
    }
}

/// Bump the graph revision when the graph's content changed this frame.
/// Cooking systems run after this and only when the revision moved.
fn track_revision(
    graph:        Res<NodeGraphState>,
    nav:          Res<GraphNavigation>,
    mouse:        Res<ButtonInput<MouseButton>>,
    keys:         Res<ButtonInput<KeyCode>>,
    mut revision: ResMut<node_graph::GraphRevision>,
    mut last:     Local<Option<u64>>,
) {
    let hash = graph.content_hash();
    // Subnet contents are not part of the hash. While one is open, any
    // click or key press counts as a change.
    let in_subnet = nav.current_subnet.is_some()
        && (mouse.get_pressed().next().is_some() || mouse.get_just_released().next().is_some()
            || keys.get_just_pressed().next().is_some());
    if *last != Some(hash) || in_subnet {
        *last = Some(hash);
        revision.0 = revision.0.wrapping_add(1);
    }
}

/// Feedback line for graph open / save.
#[derive(Resource, Default)]
struct GraphFile {
    message: String,
}

// ── Systems ───────────────────────────────────────────────────────────────────

fn update_operator_stack(
    graph:     Res<NodeGraphState>,
    revision:  Res<node_graph::GraphRevision>,
    mut stack: ResMut<OperatorStack>,
) {
    if revision.is_changed() {
        stack.rebuild(&graph.nodes, &graph.connections);
        if let Some(sel) = graph.selected_node {
            stack.selected_entry = Some(sel);
        }
    }
}

fn update_scene_hierarchy(
    graph:         Res<NodeGraphState>,
    subnets:       Res<SubnetStore>,
    revision:      Res<node_graph::GraphRevision>,
    mut hierarchy: ResMut<SceneHierarchy>,
) {
    if !revision.is_changed() { return; }

    // Update the closure to accept template parameter
    let eval_subnet = |sid: SubnetId, mesh: &MeshData, template: Option<&MeshData>| -> MeshData {
        subnets
            .get(sid)
            .map(|sg| sg.evaluate(mesh, template))
            .unwrap_or_else(|| mesh.clone())
    };

    let entries = graph.evaluate_for_scene(&eval_subnet);
    hierarchy.rebuild(entries);
}

fn update_generated_meshes(
    graph:        Res<NodeGraphState>,
    subnets:      Res<SubnetStore>,
    playback:     Res<Playback>,
    mut commands: Commands,
    mut meshes:   ResMut<Assets<Mesh>>,
    mut mats:     ResMut<Assets<StandardMaterial>>,
    query:        Query<Entity, With<GeneratedMesh>>,
    revision:     Res<node_graph::GraphRevision>,
    mut shown:    Local<Option<(f64, bool)>>,
) {
    // Cook again when the graph changed, the playhead moved or the theme
    // switched; not when the camera moves.
    let now = (playback.time, theme::is_dark());
    if !revision.is_changed() && *shown == Some(now) { return; }
    *shown = Some(now);
    for e in query.iter() { commands.entity(e).despawn(); }

    // Meshes bound to a skeleton, posed at the playhead.
    for clip in graph.display_clips() {
        let Some(skin) = &clip.skin else { continue };
        let (pos, nrm) = skin.deformed(&clip.world_pose(clip.index_at(playback.time)));
        let mut md = MeshData {
            vertices: pos.iter().map(|p| p.to_array()).collect(),
            normals:  nrm.iter().map(|n| n.to_array()).collect(),
            ..Default::default()
        };
        for f in &skin.faces {
            md.indices.extend([f[0], f[1], f[2]]);
            if !core::anim::SkinMesh::is_tri(f) { md.indices.extend([f[0], f[2], f[3]]); }
        }
        commands.spawn((
            PbrBundle {
                mesh: meshes.add(mesh_data_to_bevy(&md)),
                material: mats.add(StandardMaterial {
                    base_color: Color::srgb(0.55, 0.62, 0.7),
                    perceptual_roughness: 0.6,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                }),
                ..default()
            },
            GeneratedMesh,
        ));
    }

    let eval_subnet = |sid: SubnetId, mesh: &MeshData, template: Option<&MeshData>| -> MeshData {
        subnets
            .get(sid)
            .map(|sg| sg.evaluate(mesh, template))
            .unwrap_or_else(|| mesh.clone())
    };

    // A selected Edit Poly node takes over the viewport: it shows the mesh
    // being edited, with the selected polygons tinted.
    let stage = modelling::stage(&graph, &eval_subnet);
    if let Some(hl) = stage.as_ref().and_then(modelling::highlight_mesh) {
        commands.spawn((
            PbrBundle {
                mesh: meshes.add(mesh_data_to_bevy(&hl)),
                material: mats.add(StandardMaterial {
                    base_color: Color::srgba(1.0, 0.25, 0.15, 0.45),
                    unlit: true,
                    alpha_mode: AlphaMode::Blend,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                }),
                ..default()
            },
            GeneratedMesh,
        ));
    }
    let shown = match &stage {
        Some(s) => Some(s.mesh.to_mesh()),
        None    => graph.evaluate_for_viewport(&eval_subnet),
    };
    if let Some(md) = shown {
        if md.vertices.is_empty() { return; }
        commands.spawn((
            PbrBundle {
                mesh: meshes.add(mesh_data_to_bevy(&md)),
                material: mats.add(StandardMaterial {
                    base_color: Color::srgb(0.6, 0.6, 0.6),
                    metallic: 0.1,
                    perceptual_roughness: 0.5,
                    ..default()
                }),
                ..default()
            },
            GeneratedMesh,
        ));
    }
}

// ── Skeleton display ──────────────────────────────────────────────────────────
// Draws the clip of the viewed node (view flag, else Output) at the playhead.

fn setup_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    config.line_width = 2.5;
    config.depth_bias = -1.0;   // draw over the ground grid and meshes
}

fn draw_skeleton(
    graph:      Res<NodeGraphState>,
    playback:   Res<Playback>,
    mut gizmos: Gizmos,
) {
    for clip in graph.display_clips() {
        draw_clip_skeleton(&clip, playback.time, &mut gizmos);
    }
}

fn draw_clip_skeleton(clip: &core::anim::AnimData, time: f64, gizmos: &mut Gizmos) {
    if clip.joints.is_empty() { return; }

    let pose = clip.world_pose(clip.index_at(time));
    let pos: Vec<Vec3> = pose.iter().map(|m| m.w_axis.truncate()).collect();

    // Marker size follows the skeleton, so centimetre and metre rigs both read.
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &pos { lo = lo.min(*p); hi = hi.max(*p); }
    let r = ((hi - lo).max_element() * 0.012).max(1e-4);

    let bone_col  = Color::srgb(0.95, 0.82, 0.35);
    let joint_col = Color::srgb(0.95, 0.95, 0.95);
    for (j, joint) in clip.joints.iter().enumerate() {
        match joint.parent {
            // A helper above the bones (a character root at the origin) is
            // not a bone: no link is drawn from it.
            Some(p) => { if clip.joints[p].is_bone { gizmos.line(pos[p], pos[j], bone_col); } }
            None => {
                // Root: small axis tripod showing its orientation.
                let m = pose[j];
                let l = r * 5.0;
                gizmos.line(pos[j], pos[j] + m.x_axis.truncate().normalize_or_zero() * l, Color::srgb(1.0, 0.2, 0.2));
                gizmos.line(pos[j], pos[j] + m.y_axis.truncate().normalize_or_zero() * l, Color::srgb(0.2, 1.0, 0.2));
                gizmos.line(pos[j], pos[j] + m.z_axis.truncate().normalize_or_zero() * l, Color::srgb(0.3, 0.5, 1.0));
            }
        }
        gizmos.sphere(pos[j], Quat::IDENTITY, r, joint_col).resolution(8);
    }
}

fn apply_viewport_rect(
    vp_rect:   Res<ViewportRect>,
    windows:   Query<&Window>,
    mut cam_q: Query<&mut Camera, With<MainCamera>>,
) {
    let Ok(mut cam) = cam_q.get_single_mut() else { return };
    // The viewport pane is hidden behind another tab: draw nothing.
    let Some(rect) = vp_rect.0 else {
        if cam.is_active { cam.is_active = false; }
        return;
    };
    let Ok(window) = windows.get_single() else { return };
    let scale = window.scale_factor();
    let (win_w, win_h) = (window.physical_width(), window.physical_height());
    if win_w < 2 || win_h < 2 { return; }
    if !cam.is_active { cam.is_active = true; }

    // Kept inside the window: a viewport that sticks out is an error for the renderer.
    let x      = ((rect.min.x * scale).max(0.0) as u32).min(win_w - 1);
    let y      = ((rect.min.y * scale).max(0.0) as u32).min(win_h - 1);
    let width  = ((rect.width()  * scale) as u32).clamp(1, win_w - x);
    let height = ((rect.height() * scale) as u32).clamp(1, win_h - y);

    cam.viewport = Some(bevy::render::camera::Viewport {
        physical_position: UVec2::new(x, y),
        physical_size:     UVec2::new(width, height),
        ..default()
    });
}

fn mesh_data_to_bevy(d: &MeshData) -> Mesh {
    let mut m = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    // Polygon meshes get hard edges where faces meet sharply; see `render_buffers`.
    let (positions, normals, indices) = d.render_buffers();
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    m.insert_indices(bevy::render::mesh::Indices::U32(indices));
    m
}

// ── Scene setup ───────────────────────────────────────────────────────────────

fn setup_scene(
    mut commands: Commands,
    mut meshes:   ResMut<Assets<Mesh>>,
    mut mats:     ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3dBundle {
            transform: Transform::from_xyz(5.0, 5.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
            ..default()
        },
        MainCamera,
    ));
    commands.spawn(DirectionalLightBundle {
        directional_light: DirectionalLight {
            illuminance: 10000.0, shadows_enabled: true, ..default()
        },
        transform: Transform::from_rotation(
            Quat::from_euler(EulerRot::XYZ, -0.5, 0.5, 0.0)),
        ..default()
    });
    commands.insert_resource(AmbientLight { color: Color::WHITE, brightness: 300.0 });

    commands.spawn((
        PbrBundle {
            mesh: meshes.add(create_grid_mesh(20, 1.0)),
            material: mats.add(StandardMaterial {
                base_color: Color::srgba(0.35, 0.35, 0.35, 0.6),
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                ..default()
            }),
            ..default()
        },
        GroundGrid,
    ));

    for (dir, col) in [
        (Vec3::X, Color::srgb(1.0, 0.0, 0.0)),
        (Vec3::Y, Color::srgb(0.0, 1.0, 0.0)),
        (Vec3::Z, Color::srgb(0.0, 0.0, 1.0)),
    ] {
        commands.spawn(PbrBundle {
            mesh: meshes.add(create_axis_mesh(Vec3::ZERO, dir)),
            material: mats.add(StandardMaterial {
                base_color: col, unlit: true, ..default()
            }),
            ..default()
        });
    }
}

fn create_axis_mesh(start: Vec3, end: Vec3) -> Mesh {
    let mut m = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::LineList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![start.to_array(), end.to_array()]);
    m.insert_indices(bevy::render::mesh::Indices::U32(vec![0, 1]));
    m
}

fn create_grid_mesh(size: usize, spacing: f32) -> Mesh {
    let half = (size as f32 * spacing) / 2.0;
    let mut verts = Vec::new();
    for i in 0..=size {
        let p = i as f32 * spacing - half;
        verts.push([p, 0.0, -half]); verts.push([p, 0.0,  half]);
        verts.push([-half, 0.0, p]); verts.push([ half, 0.0, p]);
    }
    let idx: Vec<u32> = (0..verts.len() as u32).collect();
    let mut m = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::LineList,
        bevy::render::render_asset::RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, verts);
    m.insert_indices(bevy::render::mesh::Indices::U32(idx));
    m
}