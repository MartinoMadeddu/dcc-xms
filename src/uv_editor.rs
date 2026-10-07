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
    edges:     Vec<crate::uv_canvas::Edge>,
    /// Show island borders only.
    pub borders_only: bool,
    /// The wire layer as drawn last, and what it was drawn for.
    wire:      Option<egui::TextureHandle>,
    wire_key:  Option<(u64, crate::uv_canvas::View, crate::uv_canvas::Options, u64)>,
    /// How long the last redraw of the wire layer took, in milliseconds.
    wire_ms:   f32,
    /// Edge count the borders-only default was last chosen for.
    auto_for:  usize,
    island_of: Vec<usize>,
    islands:   usize,
    coverage:  f32,
    /// Tiles the layout uses, as the (u, v) of each tile's corner.
    tiles:     Vec<(i32, i32)>,
    /// Fit the view to the tiles on the next frame.
    fit:       bool,
    /// That fit takes in UVs that repeat outside the unit square.
    fit_all:   bool,
}

impl Default for UvEditorState {
    fn default() -> Self {
        Self {
            pan: egui::Vec2::ZERO, zoom: 1.0, selected: None, key: None, mesh: None,
            edges: vec![], island_of: vec![], islands: 0, coverage: 0.0, tiles: vec![], fit: true, fit_all: false,
            borders_only: false, wire: None, wire_key: None, wire_ms: 0.0, auto_for: 0,
        }
    }
}

/// More edges than this are not drawn one by one.
/// True when the tiles in use read as UDIM tiles: on the ten-wide grid,
/// none below zero, and not hundreds of them.
fn udim_like(tiles: &[(i32, i32)]) -> bool {
    tiles.len() <= 200 && tiles.iter().all(|(u, v)| (0..crate::core::uv::UDIM_ROW as i32).contains(u) && *v >= 0)
}

/// Edge count past which the editor starts in borders-only.
const AUTO_BORDERS: usize = 300_000;

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
                    .map(|(a, b, seam, tri)| crate::uv_canvas::Edge { a, b, seam, island: island_of[tri] as u32 }).collect();
                // Past this, the inside of the islands is more noise than help.
                // Decided once per layout size, so the choice made after that sticks.
                if state.auto_for != state.edges.len() {
                    state.auto_for = state.edges.len();
                    state.borders_only = state.edges.len() > AUTO_BORDERS;
                }
                state.island_of = island_of;
                state.islands = count;
                state.coverage = uv::coverage(m);
                let tiles = uv::used_tiles(m);
                // A different set of tiles: bring them all into view.
                if tiles != state.tiles { state.fit = true; }
                state.tiles = tiles;
            }
            None => { state.edges.clear(); state.island_of.clear(); state.islands = 0; state.coverage = 0.0; state.tiles.clear(); }
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
                let n = state.tiles.len().max(1);
                let used = state.coverage / n as f32 * 100.0;
                if !udim_like(&state.tiles) {
                    // UVs that run far past the unit square are a repeating
                    // texture, not a set of UDIM tiles.
                    let (u0, u1) = (state.tiles.iter().map(|t| t.0).min().unwrap_or(0), state.tiles.iter().map(|t| t.0).max().unwrap_or(0) + 1);
                    let (v0, v1) = (state.tiles.iter().map(|t| t.1).min().unwrap_or(0), state.tiles.iter().map(|t| t.1).max().unwrap_or(0) + 1);
                    ui.label(format!("{} islands, {} triangles. UVs repeat outside the unit square: u {u0} to {u1}, v {v0} to {v1}",
                        state.islands, m.indices.len() / 3));
                    if ui.small_button("Fit all").on_hover_text("Show the whole extent of the UVs").clicked() { state.fit_all = true; state.fit = true; }
                } else if n > 1 {
                    let numbers: Vec<i32> = state.tiles.iter().map(|(u, v)| uv::udim(*u, *v)).collect();
                    ui.label(format!("{} islands, {} triangles, {} UDIM tiles ({} to {}), {:.0}% of them used",
                        state.islands, m.indices.len() / 3, n, numbers[0], numbers[n - 1], used));
                } else {
                    ui.label(format!("{} islands, {} triangles, {:.0}% of the square used",
                        state.islands, m.indices.len() / 3, used));
                }
            }
            None => { ui.label("No UVs on the selected node. Add a UV Unwrap node after a mesh."); }
        }
        if ui.small_button("Fit").on_hover_text("Show every tile in use (F)").clicked() { state.fit = true; }
        ui.checkbox(&mut state.borders_only, "Borders only")
            .on_hover_text("Draw the outline of each island and leave out the edges inside. Turned on by itself for very dense layouts.");
        if state.edges.len() > 100_000 {
            ui.weak(format!("{} edges drawn in {:.0} ms", state.edges.len(), state.wire_ms));
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
    // Fit: the box around every tile in use, the unit square at least.
    if state.fit && rect.width() > 1.0 {
        state.fit = false;
        let (mut lo, mut hi) = (egui::vec2(0.0, 0.0), egui::vec2(1.0, 1.0));
        // Repeating UVs: the unit square, unless the whole extent is asked for.
        let whole = udim_like(&state.tiles) || state.fit_all;
        state.fit_all = false;
        for (u, v) in state.tiles.iter().filter(|_| whole) {
            lo = lo.min(egui::vec2(*u as f32, *v as f32));
            hi = hi.max(egui::vec2(*u as f32 + 1.0, *v as f32 + 1.0));
        }
        let size = hi - lo;
        state.zoom = (rect.width() * 0.9 / size.x).min(rect.height() * 0.86 / size.y) / side.max(1.0);
        let mid = (lo + hi) * 0.5;
        state.pan = egui::vec2(0.5 - mid.x, 0.5 - mid.y);
    }
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
        if f_key { state.fit = true; }
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

    // Grid: every tile in use, in tenths, with its UDIM number. The unit
    // square is always drawn.
    let pan = state.pan;
    let grid = egui::Color32::from_rgb(54, 57, 64);
    let mut tiles = if udim_like(&state.tiles) { state.tiles.clone() } else { vec![] };
    if !tiles.contains(&(0, 0)) { tiles.push((0, 0)); }
    let many = tiles.len() > 1;
    for (tu, tv) in &tiles {
        let (x, y) = (*tu as f32, *tv as f32);
        let square = egui::Rect::from_two_pos(to_screen([x, y], pan), to_screen([x + 1.0, y + 1.0], pan));
        if !rect.intersects(square) { continue; }
        // Tenths only when they are far enough apart to read.
        if square.width() > 120.0 {
            for k in 1..10 {
                let t = k as f32 / 10.0;
                painter.line_segment([to_screen([x + t, y], pan), to_screen([x + t, y + 1.0], pan)], egui::Stroke::new(1.0_f32, grid));
                painter.line_segment([to_screen([x, y + t], pan), to_screen([x + 1.0, y + t], pan)], egui::Stroke::new(1.0_f32, grid));
            }
        }
        painter.rect_stroke(square, 0.0, egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(120, 125, 135)));
        let label = if many || (*tu, *tv) != (0, 0) { format!("{}", uv::udim(*tu, *tv)) } else { "0,0".to_string() };
        painter.text(square.left_bottom() + egui::vec2(4.0, -4.0), egui::Align2::LEFT_BOTTOM, label,
            egui::FontId::monospace(if many { 12.0 } else { 10.0 }), egui::Color32::from_rgb(150, 155, 165));
    }
    if !many {
        painter.text(to_screen([1.0, 1.0], pan) + egui::vec2(4.0, 4.0), egui::Align2::LEFT_TOP, "1,1",
            egui::FontId::monospace(10.0), egui::Color32::from_rgb(130, 135, 145));
    }

    // Edges: drawn into a pixel buffer by the canvas and shown as one
    // texture. Inner ones faint, seams bright, the selected island in the
    // accent colour. Redrawn only when something it depends on changes.
    let ppp = ui.ctx().pixels_per_point();
    let view = crate::uv_canvas::View {
        w: (rect.width() * ppp).round().max(1.0) as usize,
        h: (rect.height() * ppp).round().max(1.0) as usize,
        scale: scale * ppp,
        ox: (rect.width() * 0.5 + (pan.x - 0.5) * scale) * ppp,
        // v runs up the screen.
        oy: (rect.height() * 0.5 + (0.5 - pan.y) * scale) * ppp,
    };
    let options = crate::uv_canvas::Options { borders_only: state.borders_only, selected: state.selected, thick: ppp >= 1.5 };
    let key = (revision, view, options, crate::theme::revision());
    if state.wire_key != Some(key) || state.wire.is_none() {
        let accent = crate::theme::accent();
        let colours = crate::uv_canvas::Colours { inner: [170, 175, 185], seam: [110, 170, 255], picked: [accent.r(), accent.g(), accent.b()] };
        let started = std::time::Instant::now();
        let pixels = crate::uv_canvas::raster(&state.edges, &view, &options, &colours);
        state.wire_ms = started.elapsed().as_secs_f32() * 1000.0;
        let image = egui::ColorImage::from_rgba_unmultiplied([view.w, view.h], &pixels);
        match &mut state.wire {
            Some(texture) => texture.set(image, egui::TextureOptions::NEAREST),
            None => state.wire = Some(ui.ctx().load_texture("uv_wire", image, egui::TextureOptions::NEAREST)),
        }
        state.wire_key = Some(key);
    }
    if let Some(texture) = &state.wire {
        painter.image(texture.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}
