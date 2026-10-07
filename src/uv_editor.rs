//! The UV Editor pane: the UV layout of the selected node, and island
//! editing when that node is a UV Edit node.

use std::sync::Arc;

use bevy::prelude::Resource;
use bevy_egui::egui;

use crate::core::uv::{self, IslandEdit};
use crate::node_graph::NodeGraphState;
use crate::types::{MeshData, NodeId, NodeType};

#[derive(Resource)]
pub struct UvEditorState {
    /// View offset in UV units, and magnification.
    pan:  egui::Vec2,
    zoom: f32,
    /// Island picked in the editor.
    pub selected: Option<u32>,
    // The layout being shown, rebuilt when the graph revision moves.
    key:       Option<(Option<NodeId>, u64)>,
    mesh:      Option<Arc<MeshData>>,
    edges:     Vec<([f32; 2], [f32; 2], bool, u32)>,
    island_of: Vec<usize>,
    islands:   usize,
    coverage:  f32,
}

impl Default for UvEditorState {
    fn default() -> Self {
        Self {
            pan: egui::Vec2::ZERO, zoom: 1.0, selected: None, key: None, mesh: None,
            edges: vec![], island_of: vec![], islands: 0, coverage: 0.0,
        }
    }
}

/// More edges than this are not drawn one by one.
const MAX_EDGES: usize = 150_000;

pub fn draw_uv_editor(
    ui:       &mut egui::Ui,
    graph:    &mut NodeGraphState,
    state:    &mut UvEditorState,
    revision: u64,
    get_mesh: &dyn Fn(&NodeGraphState) -> Option<MeshData>,
) {
    let selected_node = graph.selected_node;
    if state.key != Some((selected_node, revision)) {
        state.key = Some((selected_node, revision));
        let mesh = get_mesh(graph).filter(|m| !m.uvs.is_empty() && m.uvs.len() == m.indices.len());
        match &mesh {
            Some(m) => {
                let (island_of, count) = uv::islands(m);
                state.edges = uv::uv_edges(m).into_iter()
                    .map(|(a, b, seam, tri)| (a, b, seam, island_of[tri] as u32)).collect();
                state.island_of = island_of;
                state.islands = count;
                state.coverage = uv::coverage(m);
            }
            None => { state.edges.clear(); state.island_of.clear(); state.islands = 0; state.coverage = 0.0; }
        }
        state.mesh = mesh.map(Arc::new);
        if state.selected.map(|s| s as usize >= state.islands).unwrap_or(false) { state.selected = None; }
    }

    let editing = selected_node
        .and_then(|id| graph.nodes.iter().find(|n| n.id == id))
        .map(|n| matches!(n.node_type, NodeType::UvEdit { .. })).unwrap_or(false);
    if !editing { state.selected = None; }

    // ── Header ───────────────────────────────────────────────────────────────
    ui.horizontal_wrapped(|ui| {
        match &state.mesh {
            Some(m) => {
                ui.label(format!("{} islands, {} triangles, {:.0}% of the square used",
                    state.islands, m.indices.len() / 3, state.coverage * 100.0));
            }
            None => { ui.label("No UVs on the selected node. Add a UV Unwrap node after a mesh."); }
        }
        if ui.small_button("Fit").on_hover_text("Show the whole unit square (F)").clicked() {
            state.pan = egui::Vec2::ZERO;
            state.zoom = 1.0;
        }
        if editing {
            ui.weak("Click an island to select it, drag to move it.");
        } else {
            ui.weak("Wheel zooms, drag pans.");
        }
    });

    // ── Canvas ───────────────────────────────────────────────────────────────
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(38, 40, 46));

    let side = rect.width().min(rect.height()) * 0.86;
    let scale = (side * state.zoom).max(1.0);
    let to_screen = |uv: [f32; 2], pan: egui::Vec2| -> egui::Pos2 {
        rect.center() + egui::vec2((uv[0] - 0.5 + pan.x) * scale, -(uv[1] - 0.5 + pan.y) * scale)
    };
    let to_uv = |p: egui::Pos2, pan: egui::Vec2| -> bevy::math::Vec2 {
        let d = p - rect.center();
        bevy::math::Vec2::new(d.x / scale + 0.5 - pan.x, -d.y / scale + 0.5 - pan.y)
    };

    // Zoom about the cursor, pan, fit.
    if response.hovered() {
        let (scroll, f_key) = ui.input(|i| (i.smooth_scroll_delta.y, i.key_pressed(egui::Key::F)));
        if scroll != 0.0 {
            if let Some(p) = response.hover_pos() {
                let before = to_uv(p, state.pan);
                state.zoom = (state.zoom * (scroll * 0.004).exp()).clamp(0.1, 200.0);
                let s = (side * state.zoom).max(1.0);
                let d = p - rect.center();
                // Keep the UV under the cursor where it is.
                state.pan = egui::vec2(d.x / s + 0.5 - before.x, -d.y / s + 0.5 - before.y);
            }
        }
        if f_key { state.pan = egui::Vec2::ZERO; state.zoom = 1.0; }
    }
    let panning = response.dragged_by(egui::PointerButton::Middle)
        || response.dragged_by(egui::PointerButton::Secondary)
        || (response.dragged_by(egui::PointerButton::Primary) && (!editing || state.selected.is_none()));
    if panning {
        let d = response.drag_delta();
        state.pan += egui::vec2(d.x / scale, -d.y / scale);
    }

    // Island picking and moving, on a UV Edit node.
    if editing {
        if let (Some(m), Some(p)) = (&state.mesh, response.interact_pointer_pos()) {
            if response.drag_started_by(egui::PointerButton::Primary) || response.clicked_by(egui::PointerButton::Primary) {
                state.selected = uv::island_at(m, &state.island_of, to_uv(p, state.pan)).map(|i| i as u32);
            }
        }
        if let (Some(island), true) = (state.selected, response.dragged_by(egui::PointerButton::Primary)) {
            let d = response.drag_delta();
            let delta = [d.x / scale, -d.y / scale];
            if delta != [0.0, 0.0] {
                if let Some(node) = selected_node.and_then(|id| graph.nodes.iter_mut().find(|n| n.id == id)) {
                    if let NodeType::UvEdit { edits } = &mut node.node_type {
                        // The last edit of this island takes the move.
                        let at = match edits.iter().rposition(|e| e.island == island) {
                            Some(i) => i,
                            None => { edits.push(IslandEdit::new(island)); edits.len() - 1 }
                        };
                        edits[at].offset[0] += delta[0];
                        edits[at].offset[1] += delta[1];
                    }
                }
            }
        }
    }

    // Grid: tenths, and the unit square.
    let pan = state.pan;
    let grid = egui::Color32::from_rgb(54, 57, 64);
    for k in 0..=10 {
        let t = k as f32 / 10.0;
        painter.line_segment([to_screen([t, 0.0], pan), to_screen([t, 1.0], pan)], egui::Stroke::new(1.0_f32, grid));
        painter.line_segment([to_screen([0.0, t], pan), to_screen([1.0, t], pan)], egui::Stroke::new(1.0_f32, grid));
    }
    let square = egui::Rect::from_two_pos(to_screen([0.0, 0.0], pan), to_screen([1.0, 1.0], pan));
    painter.rect_stroke(square, 0.0, egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(120, 125, 135)));
    for (label, at) in [("0,0", [0.0, 0.0]), ("1,1", [1.0, 1.0])] {
        painter.text(to_screen(at, pan) + egui::vec2(4.0, 4.0), egui::Align2::LEFT_TOP, label,
            egui::FontId::monospace(10.0), egui::Color32::from_rgb(130, 135, 145));
    }

    // Edges: inner ones faint, seams bright, the selected island in orange.
    if state.edges.len() > MAX_EDGES {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER,
            format!("{} UV edges: too many to draw", state.edges.len()),
            egui::FontId::proportional(13.0), egui::Color32::from_rgb(200, 200, 200));
        return;
    }
    let inner  = egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(170, 175, 185, 90));
    let seam   = egui::Stroke::new(1.3_f32, egui::Color32::from_rgb(110, 170, 255));
    let picked = egui::Stroke::new(1.6_f32, egui::Color32::from_rgb(255, 150, 70));
    let view = rect.expand(2.0);
    for pass in 0..2 {
        for (a, b, is_seam, island) in &state.edges {
            let chosen = state.selected == Some(*island);
            // Seams and the selection go over the inner edges.
            if (pass == 1) != (*is_seam || chosen) { continue; }
            let (pa, pb) = (to_screen(*a, pan), to_screen(*b, pan));
            if !view.intersects(egui::Rect::from_two_pos(pa, pb)) { continue; }
            painter.line_segment([pa, pb], if chosen { picked } else if *is_seam { seam } else { inner });
        }
    }
}
