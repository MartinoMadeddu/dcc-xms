mod types;
mod node_graph;
mod scene_graph;
mod properties;
mod viewport;
mod ice;
mod usd_loader;
mod prim_inspector;

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
        .add_systems(Startup, setup_scene)
        .add_systems(Update, (
            dcc_ui,
            update_operator_stack,
            update_scene_hierarchy,
            update_generated_meshes,
            apply_viewport_rect,
            camera_controller,
            focus_camera,
            draw_origin_label,
        ))
        .run();
}

// ── Global egui theme ─────────────────────────────────────────────────────────

fn apply_grey_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    let bg_fill       = egui::Color32::from_rgb(118, 118, 118);
    let bg_fill_dark  = egui::Color32::from_rgb(100, 100, 100);
    let bg_fill_mid   = egui::Color32::from_rgb(110, 110, 110);
    let stroke_subtle = egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 80, 80));
    let text_col      = egui::Color32::from_rgb(230, 230, 230);
    let text_dim      = egui::Color32::from_rgb(190, 190, 190);

    style.visuals.panel_fill           = bg_fill;
    style.visuals.window_fill          = bg_fill;
    style.visuals.extreme_bg_color     = bg_fill_dark;
    style.visuals.faint_bg_color       = bg_fill_mid;
    style.visuals.code_bg_color        = bg_fill_dark;
    style.visuals.window_stroke        = stroke_subtle;
    style.visuals.widgets.noninteractive.bg_fill   = bg_fill;
    style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text_dim);
    style.visuals.widgets.inactive.bg_fill         = bg_fill_mid;
    style.visuals.widgets.inactive.fg_stroke       = egui::Stroke::new(1.0, text_col);
    style.visuals.widgets.hovered.bg_fill          = egui::Color32::from_rgb(140, 140, 140);
    style.visuals.widgets.hovered.fg_stroke        = egui::Stroke::new(1.0, text_col);
    style.visuals.widgets.active.bg_fill           = egui::Color32::from_rgb(150, 150, 150);
    style.visuals.widgets.active.fg_stroke         = egui::Stroke::new(1.0, egui::Color32::WHITE);
    style.visuals.widgets.open.bg_fill             = bg_fill_dark;
    style.visuals.widgets.open.fg_stroke           = egui::Stroke::new(1.0, text_col);
    style.visuals.selection.bg_fill    = egui::Color32::from_rgb(100, 130, 160);
    style.visuals.selection.stroke     = egui::Stroke::new(1.0, egui::Color32::from_rgb(160, 195, 225));
    style.visuals.hyperlink_color      = egui::Color32::from_rgb(160, 195, 230);

    ctx.set_style(style);
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
    windows:        Query<&Window>,
) {
    let ctx = contexts.ctx_mut();
    apply_grey_theme(ctx);

    let win_h = windows.get_single().map(|w| w.physical_height() as f32).unwrap_or(600.0);
    let win_w = windows.get_single().map(|w| w.physical_width()  as f32).unwrap_or(800.0);
    let scale = ctx.pixels_per_point();
    let win_w_pts = win_w / scale;
    let win_h_pts = win_h / scale;
    let mut used_right_pts = 0.0f32;

    // ── Properties panel ─────────────────────────────────────────────────────
    let props_resp = egui::SidePanel::right("properties_panel")
        .resizable(true)
        .default_width(260.0)
        .min_width(180.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                draw_properties_panel(ui, &mut graph, &*stack, &mut subnets, &nav);
            });
        });
    used_right_pts += props_resp.response.rect.width();

    // ── Scene explorer + operator stack ──────────────────────────────────────
    let scene_resp = egui::SidePanel::right("scene_panels")
        .resizable(true)
        .default_width(260.0)
        .min_width(180.0)
        .show(ctx, |ui| {
            let total_height = ui.available_height();
            let half = total_height / 2.0;
            let width = ui.available_width();

            let top_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, half));
            ui.allocate_rect(top_rect, egui::Sense::hover());
            let mut top_ui = ui.child_ui(top_rect, *ui.layout(), None);
            draw_scene_explorer(&mut top_ui, &mut hierarchy, &mut graph);

            ui.separator();

            let bot_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, half));
            ui.allocate_rect(bot_rect, egui::Sense::hover());
            let mut bot_ui = ui.child_ui(bot_rect, *ui.layout(), None);
            draw_operator_stack(&mut bot_ui, &mut stack, &mut graph);
        });
    used_right_pts += scene_resp.response.rect.width();

    // ── Node graph (full height) ──────────────────────────────────────────────
    let graph_resp = egui::SidePanel::right("node_graph_panel")
        .resizable(true)
        .default_width(680.0)
        .min_width(400.0)
        .show(ctx, |ui| {
            match nav.current_subnet {
                Some(sid) => {
                    if let Some(sg) = subnets.get_mut(sid) {
                        let subnet_name = sg.name.clone();
                        ui.heading("Node Graph");
                        if draw_breadcrumb(ui, &subnet_name) {
                            nav.current_subnet = None;
                        } else {
                            ui.label("Right-click: add  |  Shift+drag: pan  |  Esc: cancel wire");
                            ui.separator();
                            draw_subnet_graph(ui, sg);
                        }
                    } else {
                        nav.current_subnet = None;
                    }
                }
                None => {
                    ui.heading("Node Graph");
                    ui.label("Right-click/Tab: add  |  Shift+drag: pan  |  Esc: cancel wire  |  Double-click subnet: dive in");
                    ui.separator();

                    let dive = draw_node_graph(ui, &mut graph);

                    for node in graph.nodes.iter_mut() {
                        if let NodeType::Subnet { id, name } = &mut node.node_type {
                            if *id == SubnetId(usize::MAX) {
                                let new_id = subnets.create_subnet(name.clone());
                                *id = new_id;
                            }
                        }
                    }
                    if let Some(sid) = dive {
                        if sid != SubnetId(usize::MAX) {
                            nav.current_subnet = Some(sid);
                        }
                    }
                }
            }
        });
    used_right_pts += graph_resp.response.rect.width();

    // ── Primitive Inspector (bottom of viewport area) ─────────────────────────
    // Must be added BEFORE the viewport rect is finalised so egui accounts for
    // its height when we compute the remaining space.
    let insp_resp = egui::TopBottomPanel::bottom("prim_inspector_panel")
        .resizable(true)
        .default_height(220.0)
        .min_height(120.0)
        // Constrain to the viewport column only (left of all right panels)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Primitive Inspector")
                    .strong()
                    .color(egui::Color32::from_rgb(220, 220, 220)));
            });
            ui.separator();

            let get_mesh = |g: &NodeGraphState| -> Option<MeshData> {
                let id = g.selected_node?;
                let mut cache = std::collections::HashMap::new();
                let eval_subnet = |_sid: SubnetId, mesh: &MeshData| mesh.clone();
                g.eval_node(id, &mut cache, &eval_subnet)
                 .map(|r| r.into_mesh())
            };

            draw_prim_inspector(ui, &graph, &mut prim_state, &get_mesh);
        });
    let insp_h_pts = insp_resp.response.rect.height();

    // ── Viewport rect (remaining space after all panels) ──────────────────────
    vp_rect.0 = Some(egui::Rect::from_min_max(
        egui::pos2(0.0, 0.0),
        egui::pos2(win_w_pts - used_right_pts, win_h_pts - insp_h_pts),
    ));

    // ── Viewport overlay label ────────────────────────────────────────────────
    egui::Area::new("viewport_label".into())
        .fixed_pos(egui::pos2(10.0, 10.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(egui::Color32::from_rgba_premultiplied(90, 90, 90, 220))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 70, 70)))
                .rounding(6.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("🎥 VIEWPORT")
                        .strong()
                        .color(egui::Color32::from_rgb(220, 220, 220)));
                    ui.separator();
                    for line in &[
                        "Alt/Cmd + LMB: orbit",
                        "Alt/Cmd + MMB: pan",
                        "Alt/Cmd + RMB: zoom",
                        "Scroll: zoom",
                        "F: focus",
                    ] {
                        ui.label(egui::RichText::new(*line)
                            .color(egui::Color32::from_rgb(190, 190, 190)));
                    }
                });
        });
}

// ── Systems ───────────────────────────────────────────────────────────────────

fn update_operator_stack(
    graph:     Res<NodeGraphState>,
    mut stack: ResMut<OperatorStack>,
) {
    if graph.is_changed() {
        stack.rebuild(&graph.nodes, &graph.connections);
        if let Some(sel) = graph.selected_node {
            stack.selected_entry = Some(sel);
        }
    }
}

fn update_scene_hierarchy(
    graph:         Res<NodeGraphState>,
    subnets:       Res<SubnetStore>,
    mut hierarchy: ResMut<SceneHierarchy>,
) {
    if !graph.is_changed() && !subnets.is_changed() { return; }

    let eval_subnet = |sid: SubnetId, mesh: &MeshData| -> MeshData {
        subnets.get(sid)
            .map(|sg| sg.evaluate(mesh))
            .unwrap_or_else(|| mesh.clone())
    };

    let entries = graph.evaluate_for_scene(&eval_subnet);
    hierarchy.rebuild(entries);
}

fn update_generated_meshes(
    graph:        Res<NodeGraphState>,
    subnets:      Res<SubnetStore>,
    mut commands: Commands,
    mut meshes:   ResMut<Assets<Mesh>>,
    mut mats:     ResMut<Assets<StandardMaterial>>,
    query:        Query<Entity, With<GeneratedMesh>>,
) {
    if !graph.is_changed() && !subnets.is_changed() { return; }
    for e in query.iter() { commands.entity(e).despawn(); }

    let eval_subnet = |sid: SubnetId, mesh: &MeshData| -> MeshData {
        subnets.get(sid)
            .map(|sg| sg.evaluate(mesh))
            .unwrap_or_else(|| mesh.clone())
    };

    if let Some(md) = graph.evaluate_for_viewport(&eval_subnet) {
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

fn apply_viewport_rect(
    vp_rect:   Res<ViewportRect>,
    windows:   Query<&Window>,
    mut cam_q: Query<&mut Camera, With<MainCamera>>,
) {
    let Some(rect) = vp_rect.0 else { return };
    let Ok(window) = windows.get_single() else { return };
    let scale = window.scale_factor();
    let Ok(mut cam) = cam_q.get_single_mut() else { return };

    let x      = (rect.min.x * scale) as u32;
    let y      = (rect.min.y * scale) as u32;
    let width  = ((rect.width()  * scale) as u32).max(1);
    let height = ((rect.height() * scale) as u32).max(1);

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
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, d.vertices.clone());
    // Use computed normals if available, otherwise flat up-normals
    let normals: Vec<[f32; 3]> = if d.normals.len() == d.vertices.len() {
        d.normals.clone()
    } else {
        d.vertices.iter().map(|_| [0.0f32, 1.0, 0.0]).collect()
    };
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    m.insert_indices(bevy::render::mesh::Indices::U32(d.indices.clone()));
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