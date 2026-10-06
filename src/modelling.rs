//! Viewport side of the Edit Poly node: what the viewport shows while one is
//! selected, picking components with the mouse, and the selection overlay.
//!
//! While an Edit Poly node is selected the viewport shows that node, whatever
//! the view flag says: the mesh after all its operations, or the mesh
//! entering the operation whose selection is being edited.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::core::poly::{self, apply_ops, Component as Picked, PickMode, PickView, PolyMesh, PolySelection, SubLevel};
use crate::ice::SubnetStore;
use crate::node_graph::NodeGraphState;
use crate::types::{MainCamera, MeshData, NodeId, NodeType, SubnetId};

/// Gizmo settings for the modelling overlay. Unlike the skeleton, which is
/// drawn over everything, the wireframe is hidden by geometry in front of it.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct PolyGizmos;

pub fn setup_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<PolyGizmos>();
    config.line_width = 1.6;
    config.depth_bias = -0.02;   // just enough to sit on top of its own surface
}

/// The mesh and selection the viewport is editing.
pub struct Stage {
    pub node:      NodeId,
    pub mesh:      PolyMesh,
    pub selection: PolySelection,
}

/// Mesh entering a node's first input.
pub fn input_mesh(
    graph:       &NodeGraphState,
    id:          NodeId,
    eval_subnet: &impl Fn(SubnetId, &MeshData, Option<&MeshData>) -> MeshData,
) -> Option<PolyMesh> {
    let node = graph.nodes.iter().find(|n| n.id == id)?;
    let (src, out) = node.inputs.first()?.connected_output?;
    let mut cache = std::collections::HashMap::new();
    let mesh = graph.eval_node_out(src, out, &mut cache, eval_subnet)?.into_mesh();
    (!mesh.vertices.is_empty()).then(|| PolyMesh::from_mesh(&mesh))
}

/// Stage of the selected node, if it is an Edit Poly node with a mesh.
pub fn stage(
    graph:       &NodeGraphState,
    eval_subnet: &impl Fn(SubnetId, &MeshData, Option<&MeshData>) -> MeshData,
) -> Option<Stage> {
    let id   = graph.selected_node?;
    let node = graph.nodes.iter().find(|n| n.id == id)?;
    let NodeType::EditPoly { ops, pending, edit } = &node.node_type else { return None };
    let input = input_mesh(graph, id, eval_subnet)?;
    let edit  = edit.filter(|i| *i < ops.len());
    Some(Stage {
        node: id,
        mesh: apply_ops(&input, ops, edit.unwrap_or(ops.len())),
        selection: match edit {
            Some(i) => ops[i].selection.clone(),
            None    => pending.clone(),
        },
    })
}

/// Selected polygons as a mesh lifted slightly off the surface, for the
/// translucent highlight.
pub fn highlight_mesh(stage: &Stage) -> Option<MeshData> {
    let mask = stage.selection.poly_mask(&stage.mesh);
    if !mask.iter().any(|m| *m) { return None; }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for v in &stage.mesh.verts { lo = lo.min(*v); hi = hi.max(*v); }
    let lift = (hi - lo).max_element().max(1e-3) * 0.002;
    let mut verts = vec![];
    let mut polys = vec![];
    for (p, poly) in stage.mesh.polys.iter().enumerate() {
        if !mask[p] { continue; }
        let n = stage.mesh.normal(p);
        let base = verts.len() as u32;
        verts.extend(poly.iter().map(|v| (stage.mesh.verts[*v as usize] + n * lift).to_array()));
        polys.push((base..base + poly.len() as u32).collect());
    }
    Some(MeshData::from_polys(verts, polys))
}

fn subnet_eval(subnets: &SubnetStore) -> impl Fn(SubnetId, &MeshData, Option<&MeshData>) -> MeshData + '_ {
    move |sid, mesh, template| subnets.get(sid).map(|sg| sg.evaluate(mesh, template)).unwrap_or_else(|| mesh.clone())
}

fn pick_view(cam: &Camera, gt: &GlobalTransform) -> Option<(PickView, Rect)> {
    let rect = cam.logical_viewport_rect()?;
    Some((
        PickView {
            view_proj: cam.clip_from_view() * gt.compute_matrix().inverse(),
            size:      rect.size(),
            eye:       gt.translation(),
        },
        rect,
    ))
}

/// Click and box selection in the viewport.
///
/// Click replaces the selection, Ctrl adds, Shift removes. Dragging draws a
/// box. Alt is left to the camera.
pub fn pick_system(
    mut graph:    ResMut<NodeGraphState>,
    subnets:      Res<SubnetStore>,
    windows:      Query<&Window>,
    cam_q:        Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mouse:        Res<ButtonInput<MouseButton>>,
    keys:         Res<ButtonInput<KeyCode>>,
    mut contexts: EguiContexts,
    mut drag:     Local<Option<(Vec2, Vec2)>>,
) {
    let Some(stage) = stage(&graph, &subnet_eval(&subnets)) else { *drag = None; return };
    let (Ok(window), Ok((cam, gt))) = (windows.get_single(), cam_q.get_single()) else { return };
    let Some((view, rect)) = pick_view(cam, gt) else { return };
    let cursor = window.cursor_position();
    let ctx = contexts.ctx_mut();

    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight, KeyCode::SuperLeft, KeyCode::SuperRight]);
    if mouse.just_pressed(MouseButton::Left) {
        *drag = match cursor {
            Some(c) if rect.contains(c) && !alt && !ctx.is_pointer_over_area() && !ctx.wants_pointer_input() => Some((c, c)),
            _ => None,
        };
    }
    let Some((start, _)) = *drag else { return };
    let now = cursor.unwrap_or(start);
    *drag = Some((start, now));
    let boxed = start.distance(now) > 4.0;

    if mouse.pressed(MouseButton::Left) {
        if boxed {
            let r = egui::Rect::from_two_pos(egui::pos2(start.x, start.y), egui::pos2(now.x, now.y));
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("poly_marquee")));
            painter.rect_filled(r, 0.0, egui::Color32::from_rgba_unmultiplied(120, 170, 255, 30));
            painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(150, 190, 255)));
        }
        return;
    }

    // Released: apply.
    *drag = None;
    let level = stage.selection.level;
    let picked: Vec<Picked> = if boxed {
        poly::pick_rect(&stage.mesh, &view, level, start - rect.min, now - rect.min)
    } else {
        poly::pick_point(&stage.mesh, &view, level, now - rect.min, 9.0).into_iter().collect()
    };
    let mode = if keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) {
        PickMode::Add
    } else if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        PickMode::Remove
    } else {
        PickMode::Replace
    };

    if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == stage.node) {
        if let NodeType::EditPoly { ops, pending, edit } = &mut node.node_type {
            let target = match edit.filter(|i| *i < ops.len()) {
                Some(i) => &mut ops[i].selection,
                None    => pending,
            };
            target.apply_pick(&stage.mesh, &picked, mode);
        }
    }
}

/// Wireframe and selection drawn over the mesh being edited. Only the side
/// facing the camera is drawn, which is also the side that can be picked.
pub fn overlay_system(
    graph:      Res<NodeGraphState>,
    subnets:    Res<SubnetStore>,
    cam_q:      Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut gizmos: Gizmos<PolyGizmos>,
) {
    let Some(stage) = stage(&graph, &subnet_eval(&subnets)) else { return };
    let Ok((cam, gt)) = cam_q.get_single() else { return };
    let Some((view, _)) = pick_view(cam, gt) else { return };
    let mesh = &stage.mesh;
    let (vis_verts, vis_edges) = poly::visible_components(mesh, &view);
    if vis_edges.len() > 60_000 { return; }   // too dense to be useful as lines

    let sel = stage.selection.resolve(mesh);
    let wire     = Color::srgba(0.86, 0.88, 0.92, 0.55);
    let selected = Color::srgb(1.0, 0.32, 0.18);
    let point    = Color::srgb(0.35, 0.65, 1.0);

    // Edges of selected polygons are outlined too, so a polygon selection reads clearly.
    let mask = stage.selection.poly_mask(mesh);
    let mut outlined = sel.edges.clone();
    for (p, poly) in mesh.polys.iter().enumerate() {
        if !mask[p] { continue; }
        for i in 0..poly.len() { outlined.insert(poly::edge_key(poly[i], poly[(i + 1) % poly.len()])); }
    }
    for e in &vis_edges {
        let (a, b) = (mesh.verts[e[0] as usize], mesh.verts[e[1] as usize]);
        gizmos.line(a, b, if outlined.contains(e) { selected } else { wire });
    }

    if stage.selection.level == SubLevel::Vertex && mesh.verts.len() <= 8_000 {
        for (v, pos) in mesh.verts.iter().enumerate() {
            if !vis_verts[v] { continue; }
            let r = pos.distance(view.eye) * 0.006;
            let c = if sel.verts[v] { selected } else { point };
            let r = if sel.verts[v] { r * 1.6 } else { r };
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] { gizmos.line(*pos - axis * r, *pos + axis * r, c); }
        }
    }
}
