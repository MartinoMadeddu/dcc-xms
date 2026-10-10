use bevy::prelude::Vec3;
use bevy_egui::egui;
use crate::types::{ConnectionId, NodeId, NodeType, RetimeMode, SubnetId, node_type_icon};
use super::{GraphNode, NodeGraphState};

pub const NODE_WIDTH:  f32 = 180.0;
pub const NODE_HEIGHT: f32 = 50.0;
const SOCKET_RADIUS:   f32 = 4.5;
const SOCKET_HIT:      f32 = 22.0;
const NODE_ROUNDING:   f32 = 5.0;

#[allow(dead_code)]
mod xsi {
    // Light-theme colours. `theme::c` returns the dark counterpart in dark mode.
    #![allow(non_snake_case)]
    use bevy_egui::egui::Color32;
    pub fn BG() -> Color32 { crate::theme::c(100, 100, 100) }
    pub fn GRID() -> Color32 { crate::theme::c( 90,  90,  90) }
    pub fn NODE_BODY() -> Color32 { crate::theme::raised(130, 130, 130) }
    pub fn NODE_BODY_SEL() -> Color32 { crate::theme::c(110, 120, 135) }
    pub fn NODE_TITLE() -> Color32 { crate::theme::raised(105, 105, 105) }
    pub fn NODE_TITLE_SUB() -> Color32 { crate::theme::c( 80,  90, 100) }
    pub fn BORDER() -> Color32 { crate::theme::outline( 70,  70,  70) }
    pub fn BORDER_SEL() -> Color32 { crate::theme::c(180, 200, 220) }
    pub fn TEXT() -> Color32 { crate::theme::c(248, 248, 248) }
    pub fn TEXT_DIM() -> Color32 { crate::theme::c(210, 210, 210) }
    pub fn WIRE() -> Color32 { crate::theme::c(160, 160, 155) }
    pub fn WIRE_HOV() -> Color32 { crate::theme::c(220, 185,  90) }
    pub fn SOCK_IN() -> Color32 { crate::theme::c(100, 140, 100) }
    pub fn SOCK_IN_CONN() -> Color32 { crate::theme::c(130, 175, 130) }
    pub fn SOCK_IN_HOV() -> Color32 { crate::theme::c(170, 215, 170) }
    pub fn SOCK_OUT() -> Color32 { crate::theme::c(150, 120,  85) }
    pub fn SOCK_OUT_HOV() -> Color32 { crate::theme::c(200, 165, 120) }
    pub fn SOCK_OUT_DRAG() -> Color32 { crate::theme::c(230, 195, 100) }
    pub fn SEL_RECT() -> Color32 { Color32::from_rgba_premultiplied(100, 140, 200, 40) }
    pub fn SEL_RECT_BORDER() -> Color32 { crate::theme::c(120, 160, 220) }
    pub fn VIEW_FLAG() -> Color32 { crate::theme::c(100, 180, 255) } // 👁️ NEW - Blue for active view flag
    pub fn BYPASS() -> Color32 { crate::theme::c(235, 170, 60) }
    pub fn BYPASS_VEIL() -> Color32 { Color32::from_rgba_unmultiplied(128, 128, 128, 120) }
    pub fn VIEW_FLAG_HOV() -> Color32 { crate::theme::c(150, 210, 255) } // 👁️ NEW - Lighter blue on hover
}

// ============================================================================
// SOCKET POSITION HELPERS
// ============================================================================

pub fn output_socket_pos(node: &GraphNode, out_idx: usize) -> egui::Pos2 {
    let t = (out_idx + 1) as f32 / (node.outputs.len() + 1) as f32;
    egui::pos2(node.position.x + NODE_WIDTH * t, node.position.y + NODE_HEIGHT)
}

pub fn input_socket_pos(node: &GraphNode, in_idx: usize) -> egui::Pos2 {
    let t = (in_idx + 1) as f32 / (node.inputs.len() + 1) as f32;
    egui::pos2(node.position.x + NODE_WIDTH * t, node.position.y)
}

// ============================================================================
// GRAPH CANVAS
// ============================================================================

pub fn draw_node_graph(ui: &mut egui::Ui, graph: &mut NodeGraphState) -> Option<SubnetId> {
    let mut dive_into: Option<SubnetId> = None;

    let (response, painter) = ui.allocate_painter(
        egui::Vec2::new(ui.available_width(), ui.available_height()),
        egui::Sense::click_and_drag(),
    );
    let canvas_rect = response.rect;
    let pan  = graph.pan_offset;
    let zoom = graph.zoom;

    // to_screen: canvas-space → screen-space, applying zoom around canvas origin
    let to_screen = |p: egui::Pos2| {
        canvas_rect.min + egui::vec2(p.x * zoom + pan.x, p.y * zoom + pan.y)
    };
    // to_canvas: screen-space → canvas-space
    let to_canvas = |p: egui::Pos2| {
        egui::pos2((p.x - canvas_rect.min.x - pan.x) / zoom,
                   (p.y - canvas_rect.min.y - pan.y) / zoom)
    };

    // ── Frame every node ─────────────────────────────────────────────────────
    let frame_key = response.hovered() && !ui.ctx().wants_keyboard_input()
        && ui.input(|i| i.key_pressed(egui::Key::F) || i.key_pressed(egui::Key::A));
    if (graph.frame_request || frame_key) && canvas_rect.width() > 40.0 && !graph.nodes.is_empty() {
        graph.frame_request = false;
        let mut bounds = egui::Rect::NOTHING;
        for node in &graph.nodes {
            bounds = bounds.union(egui::Rect::from_min_size(node.position, egui::vec2(NODE_WIDTH, NODE_HEIGHT)));
        }
        let room = canvas_rect.size() - egui::vec2(60.0, 70.0);
        // Not so small that the names cannot be read: a graph too large
        // for that is shown from its middle.
        graph.zoom = (room.x / bounds.width()).min(room.y / bounds.height()).clamp(0.45, 1.0);
        let middle = bounds.center().to_vec2() * graph.zoom;
        graph.pan_offset = canvas_rect.size() * 0.5 - middle;
        ui.ctx().request_repaint();
    }

    // ── Add-node menu: Tab, right-click, or right-click on an output ─────────
    let mut opened_now = false;
    if response.hovered() && ui.input(|i| i.key_pressed(egui::Key::Tab)) {
        let cursor = ui.input(|i| i.pointer.hover_pos()).unwrap_or(canvas_rect.center());
        graph.tab_menu_screen_pos = Some(cursor);
        graph.tab_menu_canvas_pos = Some(to_canvas(cursor));
        graph.menu_from = None;
        opened_now = true;
    }
    // A right-click anywhere on the canvas. Over an output socket the menu
    // belongs to that socket; over a wire it is left to the wire.
    let right_click = ui.input(|i| if i.pointer.secondary_clicked() { i.pointer.interact_pos() } else { None })
        .filter(|p| canvas_rect.contains(*p) && ui.rect_contains_pointer(canvas_rect));
    if let Some(cursor) = right_click {
        let socket = graph.nodes.iter().find_map(|node| (0..node.outputs.len()).find(|o| {
            to_screen(output_socket_pos(node, *o)).distance(cursor) <= SOCKET_HIT * 0.5 + 2.0
        }).map(|o| (node.id, o)));
        if socket.is_some() || response.secondary_clicked() {
            graph.tab_menu_screen_pos = Some(cursor);
            graph.tab_menu_canvas_pos = Some(to_canvas(cursor));
            graph.menu_from = socket;
            graph.connecting_from = None;
            opened_now = true;
        }
    }
    if let Some(screen_pos) = graph.tab_menu_screen_pos {
        let canvas_pos = graph.tab_menu_canvas_pos.unwrap_or_default();
        let before: Vec<NodeId> = graph.nodes.iter().map(|n| n.id).collect();
        let mut close  = false;
        let area_resp = egui::Area::new(egui::Id::new("tab_add_node"))
            .fixed_pos(screen_pos)
            .order(egui::Order::Foreground)
            .constrain(true)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(180.0);
                    if let Some((from, _)) = graph.menu_from {
                        if let Some(node) = graph.nodes.iter().find(|n| n.id == from) {
                            ui.label(egui::RichText::new(format!("After {}", node.name)).small());
                            ui.separator();
                        }
                    }
                    if add_node_menu(ui, graph, canvas_pos) { close = true; }
                });
            });
        // Where the new node goes: under the socket it was asked from and
        // wired to it, or centred on the place that was clicked.
        let fresh: Vec<NodeId> = graph.nodes.iter().map(|n| n.id).filter(|id| !before.contains(id)).collect();
        if let Some(new_id) = fresh.first().copied() {
            let from = graph.menu_from.and_then(|(id, out)| graph.nodes.iter().find(|n| n.id == id).map(|n| (id, out, n.position)));
            let mut at = match from {
                Some((_, _, pos)) => pos + egui::vec2(0.0, NODE_HEIGHT + 50.0),
                None => canvas_pos - egui::vec2(NODE_WIDTH * 0.5, NODE_HEIGHT * 0.5),
            };
            // Step sideways until the place is free.
            let taken = |p: egui::Pos2, graph: &NodeGraphState| graph.nodes.iter().any(|n| n.id != new_id
                && (n.position.x - p.x).abs() < NODE_WIDTH + 10.0 && (n.position.y - p.y).abs() < NODE_HEIGHT + 10.0);
            for _ in 0..40 { if !taken(at, graph) { break; } at.x += NODE_WIDTH + 30.0; }
            if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == new_id) { node.position = at; }
            if let Some((src, out, _)) = from {
                let has_input = graph.nodes.iter().find(|n| n.id == new_id).map(|n| !n.inputs.is_empty()).unwrap_or(false);
                if has_input { graph.add_connection(src, out, new_id, 0); }
            }
            graph.selected_node = Some(new_id);
            graph.selected_nodes = vec![new_id];
            // Never leave a new node out of sight: move the view just enough.
            let rect = egui::Rect::from_min_size(to_screen(at), egui::vec2(NODE_WIDTH, NODE_HEIGHT) * zoom);
            let view = canvas_rect.shrink(12.0);
            let mut shift = egui::Vec2::ZERO;
            if rect.max.x > view.max.x { shift.x = view.max.x - rect.max.x; }
            if rect.min.x + shift.x < view.min.x { shift.x = view.min.x - rect.min.x; }
            if rect.max.y > view.max.y { shift.y = view.max.y - rect.max.y; }
            if rect.min.y + shift.y < view.min.y { shift.y = view.min.y - rect.min.y; }
            graph.pan_offset += shift;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) { close = true; }
        if !opened_now && ui.input(|i| i.pointer.any_click()) && !area_resp.response.contains_pointer()
            && !ui.ctx().memory(|m| m.any_popup_open()) {
            close = true;
        }
        if close {
            graph.tab_menu_screen_pos = None;
            graph.tab_menu_canvas_pos = None;
            graph.menu_from = None;
        }
    }

    let grid_spacing = 50.0 * zoom;
    let offset_x = pan.x % grid_spacing;
    let offset_y = pan.y % grid_spacing;
    let mut x = canvas_rect.min.x + offset_x;
    while x < canvas_rect.max.x {
        painter.line_segment(
            [egui::pos2(x, canvas_rect.min.y), egui::pos2(x, canvas_rect.max.y)],
            egui::Stroke::new(1.0, xsi::GRID()));
        x += grid_spacing;
    }
    let mut y = canvas_rect.min.y + offset_y;
    while y < canvas_rect.max.y {
        painter.line_segment(
            [egui::pos2(canvas_rect.min.x, y), egui::pos2(canvas_rect.max.x, y)],
            egui::Stroke::new(1.0, xsi::GRID()));
        y += grid_spacing;
    }

    // ── Connections ──────────────────────────────────────────────────────────
    let mut hovered_conn: Option<ConnectionId> = None;
    for conn in &graph.connections.clone() {
        if let (Some(fn_), Some(tn)) = (
            graph.nodes.iter().find(|n| n.id == conn.from_node),
            graph.nodes.iter().find(|n| n.id == conn.to_node),
        ) {
            let fp  = to_screen(output_socket_pos(fn_, conn.from_output));
            let tp  = to_screen(input_socket_pos(tn,   conn.to_input));
            let is_sel = graph.selected_connection == Some(conn.id);
            if let Some(ptr) = response.hover_pos() {
                if is_near_bezier(ptr, fp, tp, 10.0) { hovered_conn = Some(conn.id); }
            }
            draw_wire(&painter, fp, tp,
                hovered_conn == Some(conn.id),
                is_sel);
        }
    }
    if let Some(cid) = hovered_conn {
        // LMB click → select wire, clear node selection
        if response.clicked() {
            graph.selected_connection = Some(cid);
            graph.selected_nodes.clear();
            graph.selected_node = None;
        }
        // RMB → delete immediately (existing behaviour)
        if response.secondary_clicked() { graph.remove_connection(cid); }
    }

    // In-progress wire
    if let Some((fid, fo)) = graph.connecting_from {
        if let Some(fn_) = graph.nodes.iter().find(|n| n.id == fid) {
            let fp  = to_screen(output_socket_pos(fn_, fo));
            let ptr = ui.input(|i| i.pointer.hover_pos());
            if let Some(ptr) = ptr { draw_wire(&painter, fp, ptr, false, false); }
        }
    }

    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        graph.connecting_from = None;
    }

    // ── Delete selected nodes or wire ─────────────────────────────────────────
    // Use canvas_rect.contains(pointer) instead of response.hovered() so that
    // the delete key fires even when the pointer is over a node widget.
    let ptr_in_canvas = ui.input(|i| i.pointer.hover_pos())
        .map(|p| canvas_rect.contains(p))
        .unwrap_or(false);
    if ptr_in_canvas && ui.input(|i| i.key_pressed(egui::Key::Delete)
        || i.key_pressed(egui::Key::Backspace))
    {
        graph.delete_selected();
    }

    // ── Down arrow: into the selected ICE node, as a double-click does ───────
    if ptr_in_canvas && !ui.ctx().wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
        let selected = graph.selected_node.and_then(|id| graph.nodes.iter().find(|n| n.id == id));
        if let Some(NodeType::Subnet { id: sid, .. }) = selected.map(|n| &n.node_type) {
            dive_into = Some(*sid);
        }
    }

    // ── Nodes ─────────────────────────────────────────────────────────────────
    let nodes_clone = graph.nodes.clone();
    for node in &nodes_clone {
        if let Some(id) = draw_node(ui, &painter, graph, node, &to_screen, &to_canvas, canvas_rect) {
            dive_into = Some(id);
        }
    }

    // ── Marquee select ────────────────────────────────────────────────────────
    let is_panning   = ui.input(|i| i.modifiers.shift) || ui.input(|i| i.pointer.middle_down());
    let is_wiring    = graph.connecting_from.is_some();
    let is_dragging  = graph.dragging_node.is_some();

    if response.drag_started()
        && !is_panning && !is_wiring && !is_dragging
        && graph.tab_menu_screen_pos.is_none()
    {
        if let Some(pos) = response.interact_pointer_pos() {
            graph.marquee_start = Some(to_canvas(pos));
        }
    }

    if response.dragged() && !is_panning && !is_wiring && !is_dragging {
        if let (Some(start), Some(cur)) = (
            graph.marquee_start,
            response.interact_pointer_pos().map(|p| to_canvas(p)),
        ) {
            let r = egui::Rect::from_two_pos(start, cur);
            // Highlight nodes inside the marquee
            let newly_selected: Vec<NodeId> = graph.nodes.iter()
                .filter(|n| {
                    let node_rect = egui::Rect::from_min_size(
                        n.position, egui::vec2(NODE_WIDTH, NODE_HEIGHT));
                    r.intersects(node_rect)
                })
                .map(|n| n.id)
                .collect();
            graph.selected_nodes = newly_selected;

            // Draw the marquee rectangle in screen space
            let sr = egui::Rect::from_two_pos(to_screen(start), to_screen(cur));
            painter.rect_filled(sr, 2.0, xsi::SEL_RECT());
            painter.rect_stroke(sr, 2.0, egui::Stroke::new(1.0, xsi::SEL_RECT_BORDER()));
        }
    }

    if response.drag_stopped() {
        if graph.marquee_start.is_some() {
            // Commit marquee selection — selected_nodes already set above.
            // If nothing was selected, clear.
            graph.marquee_start = None;
            // Set selected_node to first in selection for properties panel sync
            graph.selected_node = graph.selected_nodes.first().copied();
        }
    }

    // Cancel wire if mouse released over empty canvas
    if graph.connecting_from.is_some() && response.drag_stopped() {
        graph.connecting_from = None;
    }

    // ── Click on empty canvas → clear all selection ───────────────────────────
    if response.clicked() && !is_panning && !is_wiring && hovered_conn.is_none() {
        graph.selected_nodes.clear();
        graph.selected_node = None;
        graph.selected_connection = None;
    }

    // ── Pan (shift+drag or MMB) ───────────────────────────────────────────────
    if response.dragged() && is_panning {
        graph.pan_offset += response.drag_delta();
    }

    // ── Zoom (scroll wheel, zoom toward cursor) ───────────────────────────────
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let factor     = 1.0 + scroll * 0.002;
            let cursor_s   = ui.input(|i| i.pointer.hover_pos()).unwrap_or(canvas_rect.center());
            // cursor in canvas space before zoom change
            let cursor_c   = to_canvas(cursor_s);
            graph.zoom     = (graph.zoom * factor).clamp(0.15, 4.0);
            // Recompute pan so cursor_c stays under cursor_s after zoom
            graph.pan_offset = egui::vec2(
                cursor_s.x - canvas_rect.min.x - cursor_c.x * graph.zoom,
                cursor_s.y - canvas_rect.min.y - cursor_c.y * graph.zoom,
            );
        }
    }

    dive_into
}

// ============================================================================
// SINGLE NODE
// ============================================================================

/// One name per node type, the same in the menu, on the node and in the
/// properties.
fn node_type_label(t: &NodeType) -> &'static str { crate::types::node_type_label(t) }

fn draw_node(
    ui:          &mut egui::Ui,
    painter:     &egui::Painter,
    graph:       &mut NodeGraphState,
    node:        &GraphNode,
    to_screen:   &impl Fn(egui::Pos2) -> egui::Pos2,
    to_canvas:   &impl Fn(egui::Pos2) -> egui::Pos2,
    _canvas_rect: egui::Rect,
) -> Option<SubnetId> {
    let mut dive: Option<SubnetId> = None;
    let zoom   = graph.zoom;
    let np     = to_screen(node.position);
    let rect   = egui::Rect::from_min_size(np, egui::vec2(NODE_WIDTH * zoom, NODE_HEIGHT * zoom));
    let id     = node.id;
    let is_sel = graph.selected_node == Some(id) || graph.selected_nodes.contains(&id);
    let is_sub = matches!(node.node_type, NodeType::Subnet { .. });

    // ── Node body ─────────────────────────────────────────────────────────────
    // As the ICE nodes: a grey ring around the node (white when selected),
    // one colour over the whole node, a dark edge, the name in bold over the
    // type.
    // One colour over the whole node, by the add-menu category it is in.
    let (body, ink, ink_dim) = node_colours(&node.node_type);
    let _ = is_sub;
    let ring = rect.expand(2.0 * zoom.max(0.5));
    painter.rect_filled(ring, (NODE_ROUNDING + 2.0) * zoom,
        if is_sel { egui::Color32::WHITE } else { egui::Color32::from_rgb(105, 105, 105) });
    painter.rect_filled(rect, NODE_ROUNDING * zoom, body);

    let title_h    = 32.0 * zoom;
    let title_rect = egui::Rect::from_min_size(np, egui::vec2(NODE_WIDTH * zoom, title_h));
    painter.rect_stroke(rect, NODE_ROUNDING * zoom, egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(38, 38, 38)));

    // Icon, then the name in bold (drawn twice, a hair apart), the type
    // beneath in PascalCase, all from the left, after the bypass ring.
    // The node's icon leads the name, as in the add menu.
    let text_x = np.x + 24.0 * zoom;
    painter.text(egui::pos2(text_x, np.y + 12.0 * zoom), egui::Align2::LEFT_CENTER, node_type_icon(&node.node_type),
        egui::FontId::proportional(12.0 * zoom), ink);
    let name_x = text_x + 17.0 * zoom;
    for dx in [0.0, 0.6] {
        painter.text(egui::pos2(name_x + dx * zoom, np.y + 12.0 * zoom), egui::Align2::LEFT_CENTER, &node.name,
            egui::FontId::proportional(12.5 * zoom), ink);
    }
    let type_name: String = node_type_label(&node.node_type).split_whitespace()
        .map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default() })
        .collect();
    painter.text(egui::pos2(name_x, np.y + 25.0 * zoom), egui::Align2::LEFT_CENTER, type_name,
        egui::FontId::proportional(9.5 * zoom), ink_dim);

    let dr = ui.allocate_rect(title_rect, egui::Sense::click_and_drag());

    // ============================================================================
    // 👁️ NEW - VIEW FLAG BUTTON
    // ============================================================================
    // Draw view flag button in top-right corner of title bar
    let has_view_flag = graph.has_view_flag(id);
    let eye_icon = if has_view_flag { "👁" } else { "○" };
    let eye_size = 16.0 * zoom;
    let eye_pos = egui::pos2(np.x + NODE_WIDTH * zoom - eye_size - 4.0 * zoom, np.y + 4.0 * zoom);
    let eye_rect = egui::Rect::from_min_size(eye_pos, egui::vec2(eye_size, eye_size));
    
    let eye_response = ui.allocate_rect(eye_rect, egui::Sense::click());
    
    // Draw the eye button
    let eye_color = if has_view_flag {
        if eye_response.hovered() { xsi::VIEW_FLAG_HOV() } else { xsi::VIEW_FLAG() }
    } else {
        if eye_response.hovered() { ink } else { ink_dim }
    };
    
    painter.text(
        egui::pos2(eye_pos.x + eye_size / 2.0, eye_pos.y + eye_size / 2.0),
        egui::Align2::CENTER_CENTER,
        eye_icon,
        egui::FontId::proportional(12.0 * zoom),
        eye_color
    );
    
// Handle click - toggle view flag
    let click_at = if dr.clicked() { dr.interact_pointer_pos() } else { None };
    let on_eye = click_at.map(|p| eye_rect.contains(p)).unwrap_or(false);
    if eye_response.clicked() || on_eye {
        graph.toggle_view_flag(id);
    }
    
    // Tooltip - simpler API
    if eye_response.hovered() {
        let tooltip_text = if has_view_flag {
            "Viewport showing this node (click to revert to Output)"
        } else {
            "Click to view this node's output in viewport"
        };
        eye_response.on_hover_text(tooltip_text);
    }
    // ============================================================================
    // END VIEW_FLAG_BUTTON
    // ============================================================================

    // ── Bypass button, left of the title ─────────────────────────────────────
    let mut on_bypass = false;
    if !matches!(node.node_type, NodeType::Output) {
        let by_rect = egui::Rect::from_min_size(
            egui::pos2(np.x + 4.0 * zoom, np.y + 4.0 * zoom), egui::vec2(eye_size, eye_size));
        let by = ui.allocate_rect(by_rect, egui::Sense::click());
        let over = ui.input(|i| i.pointer.hover_pos()).map(|p| by_rect.contains(p)).unwrap_or(false);
        let col = if node.bypassed { xsi::BYPASS() }
                  else if over { ink } else { ink_dim };
        // Drawn by hand: a ring, with a bar through it when bypassed.
        let c = by_rect.center();
        let r = 4.5 * zoom;
        painter.circle_stroke(c, r, egui::Stroke::new(1.3 * zoom, col));
        if node.bypassed {
            let d = egui::vec2(r, -r) * 1.25;
            painter.line_segment([c - d, c + d], egui::Stroke::new(1.6 * zoom, col));
        }
        on_bypass = click_at.map(|p| by_rect.contains(p)).unwrap_or(false);
        if by.clicked() || on_bypass { graph.toggle_bypass(id); }
        by.on_hover_text(if node.bypassed {
            "Bypassed: the first input passes through unchanged. Click to turn the node back on"
        } else {
            "Bypass this node"
        });
    }
    if node.bypassed {
        painter.rect_filled(rect, NODE_ROUNDING * zoom, xsi::BYPASS_VEIL());
    }

    // ── Output sockets ────────────────────────────────────────────────────────
    for (i, out) in node.outputs.iter().enumerate() {
        let t  = (i + 1) as f32 / (node.outputs.len() + 1) as f32;
        let sp = egui::pos2(np.x + NODE_WIDTH * zoom * t, np.y + NODE_HEIGHT * zoom);
        let hit = egui::Rect::from_center_size(sp, egui::vec2(SOCKET_HIT, SOCKET_HIT));
        let sr  = ui.allocate_rect(hit, egui::Sense::click_and_drag());

        let is_wiring = graph.connecting_from == Some((id, i));
        let r = SOCKET_RADIUS * zoom * if sr.hovered() || is_wiring { 1.3 } else { 1.0 };
        painter.circle_filled(sp, r + 1.0, egui::Color32::from_rgb(30, 30, 30));
        painter.circle_filled(sp, r,
            if is_wiring         { xsi::SOCK_OUT_DRAG() }
            else if sr.hovered() { xsi::SOCK_OUT_HOV()  }
            else                 { xsi::SOCK_OUT()       });
        painter.text(egui::pos2(sp.x, sp.y + SOCKET_RADIUS * zoom + 3.0),
            egui::Align2::CENTER_TOP, &out.name,
            egui::FontId::proportional(9.0 * zoom), xsi::TEXT_DIM());

        // The left button draws a wire. The right button opens the add-node
        // menu for this socket, handled with the canvas.
        let left = ui.input(|i| i.pointer.primary_down());
        if left && (sr.drag_started_by(egui::PointerButton::Primary) || (sr.is_pointer_button_down_on() && graph.connecting_from.is_none())) {
            graph.connecting_from = Some((id, i));
        }
    }

    // ── Input sockets ─────────────────────────────────────────────────────────
    for (i, inp) in node.inputs.iter().enumerate() {
        let t  = (i + 1) as f32 / (node.inputs.len() + 1) as f32;
        let sp = egui::pos2(np.x + NODE_WIDTH * zoom * t, np.y);
        let hit = egui::Rect::from_center_size(sp, egui::vec2(SOCKET_HIT, SOCKET_HIT));
        let sr  = ui.allocate_rect(hit, egui::Sense::drag());

        let r = SOCKET_RADIUS * zoom * if sr.hovered() { 1.3 } else { 1.0 };
        painter.circle_filled(sp, r + 1.0, egui::Color32::from_rgb(30, 30, 30));
        painter.circle_filled(sp, r,
            if sr.hovered()                        { xsi::SOCK_IN_HOV()  }
            else if inp.connected_output.is_some() { xsi::SOCK_IN_CONN() }
            else                                   { xsi::SOCK_IN()      });
        painter.text(egui::pos2(sp.x, sp.y - SOCKET_RADIUS * zoom - 3.0),
            egui::Align2::CENTER_BOTTOM, &inp.name,
            egui::FontId::proportional(9.0 * zoom), xsi::TEXT_DIM());

        if sr.hovered() && ui.input(|inp| inp.pointer.primary_released()) {
            if let Some((fn_, fo)) = graph.connecting_from {
                if fn_ != id { graph.add_connection(fn_, fo, id, i); }
                graph.connecting_from = None;
            }
        }
    }

    // ── Body double-click → rename ────────────────────────────────────────────
    let body_rect = egui::Rect::from_min_size(
        egui::pos2(np.x, np.y + title_h),
        egui::vec2(NODE_WIDTH * zoom, (NODE_HEIGHT - 32.0) * zoom));
    let body_sense = ui.allocate_rect(body_rect, egui::Sense::click());
    if body_sense.double_clicked() {
        graph.renaming_node = Some(id);
        graph.rename_buffer = node.name.clone();
    }

    if graph.renaming_node == Some(id) {
        let edit_rect = egui::Rect::from_min_size(
            egui::pos2(np.x + 4.0, np.y + title_h + 2.0),
            egui::vec2(NODE_WIDTH * zoom - 8.0, 22.0));
        let r = ui.put(edit_rect,
            egui::TextEdit::singleline(&mut graph.rename_buffer)
                .font(egui::FontId::proportional(11.0)));
        r.request_focus();
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let new_name = graph.rename_buffer.clone();
            if let Some(n) = graph.nodes.iter_mut().find(|n| n.id == id) { n.name = new_name; }
            graph.renaming_node = None;
        } else if ui.input(|i| i.key_pressed(egui::Key::Escape))
            || (!r.has_focus() && !r.gained_focus())
        {
            graph.renaming_node = None;
        }
    }

    // ── Title bar drag / click ────────────────────────────────────────────────
    if dr.clicked() && !on_eye && !on_bypass {
        // Clicking a node: select it. If not shift-held, clear multi-selection.
        if !ui.input(|i| i.modifiers.shift) {
            graph.selected_nodes.clear();
        }
        graph.selected_node = Some(id);
        if !graph.selected_nodes.contains(&id) {
            graph.selected_nodes.push(id);
        }
    }
    if dr.double_clicked() {
        if let NodeType::Subnet { id: sid, .. } = &node.node_type { dive = Some(*sid); }
    }
    if dr.drag_started() && graph.connecting_from.is_none() {
        // If dragging a node not in the current selection, replace selection
        if !graph.selected_nodes.contains(&id) {
            graph.selected_nodes = vec![id];
            graph.selected_node  = Some(id);
        }
        graph.dragging_node = Some(id);
        graph.drag_offset   = dr.interact_pointer_pos().unwrap_or_default() - np;
    }
    if dr.dragged() && graph.dragging_node == Some(id) {
        if let Some(ptr) = dr.interact_pointer_pos() {
            let new_pos = to_canvas(ptr - graph.drag_offset);
            if let Some(n) = graph.nodes.iter_mut().find(|n| n.id == id) {
                let delta = new_pos - n.position;
                n.position = new_pos;
                // Move all other selected nodes by the same delta
                if graph.selected_nodes.len() > 1 {
                    let ids: Vec<NodeId> = graph.selected_nodes.iter()
                        .copied()
                        .filter(|&i| i != id)
                        .collect();
                    for other_id in ids {
                        if let Some(other) = graph.nodes.iter_mut().find(|n| n.id == other_id) {
                            other.position += delta;
                        }
                    }
                }
            }
        }
    }
    if dr.drag_stopped() { graph.dragging_node = None; }

    dive
}

/// The palette node bodies take, by add-menu category.
const CATEGORY_COLOURS: [(&str, [u8; 3]); 7] = [
    // The pastel palette at 80%, one colour per category.
    ("Create",     [0xAE, 0xC0, 0x96]),   // light green   (#DAF0BB)
    ("Modify",     [0x48, 0x5A, 0x2E]),   // olive         (#5A703A)
    ("UV",         [0xC0, 0xB2, 0x88]),   // sand          (#F0DEAA)
    ("File",       [0xC0, 0x9E, 0x96]),   // salmon        (#F0C5BB)
    ("Primitives", [0x55, 0x5F, 0x7C]),   // slate blue    (#6A779B)
    ("Animation",  [0x96, 0xA2, 0xC0]),   // light blue    (#BCCAF0)
    ("Mocap",      [0xAB, 0x98, 0xBA]),   // mauve         (#D6BEE8)
];

/// A node's body colour, and the text colours that read on it: black on
/// the light colours, white on the dark ones. Nodes in no category (Output)
/// keep the theme's node grey.
fn node_colours(t: &NodeType) -> (egui::Color32, egui::Color32, egui::Color32) {
    use std::sync::OnceLock;
    type Table = Vec<(std::mem::Discriminant<NodeType>, [u8; 3])>;
    static TABLE: OnceLock<Table> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        catalog().into_iter().flat_map(|(_, category, nodes)| {
            let colour = CATEGORY_COLOURS.iter().find(|(c, _)| *c == category).map(|(_, rgb)| *rgb);
            nodes.into_iter().filter_map(move |(n, _)| colour.map(|c| (std::mem::discriminant(&n), c)))
        }).collect()
    });
    let Some(&(_, [r, g, b])) = table.iter().find(|(d, _)| *d == std::mem::discriminant(t)) else {
        return (xsi::NODE_BODY(), xsi::TEXT(), xsi::TEXT_DIM());
    };
    let light = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32 > 125.0;
    let (ink, dim) = if light {
        (egui::Color32::from_rgb(15, 15, 15), egui::Color32::from_rgb(70, 70, 70))
    } else {
        (egui::Color32::from_rgb(248, 248, 248), egui::Color32::from_rgb(215, 215, 215))
    };
    (egui::Color32::from_rgb(r, g, b), ink, dim)
}

// ============================================================================
// ADD-NODE MENU
// ============================================================================

/// Every node the menu offers, by category: the node as it is made, and
/// one line on what it does. Names and icons come from the node types, so
/// the menu, the node and the properties say the same thing.
pub fn catalog() -> Vec<(&'static str, &'static str, Vec<(NodeType, &'static str)>)> {
    use crate::types::SplitPick;
    vec![
        ("◼", "Create", vec![
            (NodeType::CreateCube { size: 1.0 }, "A cube"),
            (NodeType::CreateSphere { radius: 0.5, segments: 32 }, "A sphere"),
            (NodeType::CreateGrid { rows: 10, cols: 10, size: 2.0 }, "A flat grid of quads"),
            (NodeType::TestClip { seconds: 4.0, fps_num: 30, fps_den: 1 }, "A walk on the spot, to try the animation nodes"),
        ]),
        ("📂", "File", vec![
            (NodeType::LoadUsd { path: String::new() }, "A USD file, as packed primitives"),
            (NodeType::WriteUsd { path: String::new() }, "What the network changed in its USD stages, as an override layer"),
            (NodeType::LoadFbx { path: String::new(), take: 0 }, "A skeleton and one take from an FBX file, with its skinned mesh"),
            (NodeType::LoadFbxDir { dir: String::new(), index: 0, take: 0 }, "One FBX of a folder at a time; Write FBX can run over all of them"),
            (NodeType::LoadFbxMesh { path: String::new() }, "Every mesh of an FBX file, as packed primitives: a set to collide with"),
            (NodeType::WriteFbx { path: crate::types::DEFAULT_WRITE_PATH.into(), mesh: false }, "Write the clip as FBX, with the skeleton it came in with"),
        ]),
        ("🔨", "Modify", vec![
            (NodeType::Transform { translation: Vec3::ZERO, rotation: Vec3::ZERO, scale: Vec3::ONE }, "Translate, rotate and scale anything: a mesh, primitives or a clip"),
            (NodeType::EditPoly { ops: vec![], pending: Default::default(), edit: None, auto_collapse: false }, "Polygon modelling in one node"),
            (NodeType::Merge, "Two meshes as one"),
            (NodeType::ScatterPoints { count: 100, seed: 42 }, "Points spread over a surface"),
            (NodeType::CopyToPoints, "A copy of the first input on every point of the second"),
            (NodeType::Subnet { id: SubnetId(usize::MAX), name: "ICE".into() }, "A graph inside a node"),
        ]),
        ("📦", "Primitives", vec![
            (NodeType::PickPrims { pattern: String::new() }, "Choose packed primitives for the nodes after this one to work on"),
            (NodeType::PrunePrims { pattern: String::new(), keep: false }, "Remove packed primitives, or keep only some"),
            (NodeType::UnpackPrims, "Merge packed primitives into one mesh"),
        ]),
        ("🗺", "UV", vec![
            (NodeType::UvUnwrap { method: crate::core::uv::UvMethod::Conformal, angle: 66.0, margin: 0.02, axis: 1, tiles: 1 }, "Make texture coordinates"),
            (NodeType::UvTransform { offset: [0.0; 2], rotate: 0.0, scale: [1.0; 2] }, "Translate, rotate and scale the whole UV layout"),
            (NodeType::UvEdit { edits: vec![] }, "Move single UV islands, in the UV Editor"),
        ]),
        ("🎬", "Animation", vec![
            (NodeType::RenameJoints { find: String::new(), replace: String::new(), strip_namespace: true, prefix: String::new() }, "Rename joints by pattern"),
            (NodeType::TrimClip { head: 0, tail: 0 }, "Cut frames from the start and the end"),
            (NodeType::Retime { fps_num: 30, fps_den: 1, mode: RetimeMode::Resample }, "Change the frame rate"),
            (NodeType::SetTimecode { hours: 1, minutes: 0, seconds: 0, frames: 0, drop_frame: false }, "Change where the clip starts"),
            (NodeType::TimeWarp { speed: 1.0, reverse: false }, "Play faster, slower or backwards"),
            (NodeType::BlendClips { blend: 15, align: true }, "One clip, then another, blended"),
            (NodeType::LoopClip { blend: 15 }, "Ease the end into the start, so the clip cycles"),
            (NodeType::MirrorClip, "Swap left and right"),
            (NodeType::SmoothClip { radius: 3, amount: 1.0, translations: true }, "Filter out jitter"),
            (NodeType::InPlace { keep_height: true, to_root: false }, "Hold the hips over their starting point"),
            (NodeType::FloorClip { height: 0.0 }, "Put the lowest point on the floor"),
            (NodeType::PruneJoints { words: "finger, thumb".into() }, "Remove joints and everything below them"),
        ]),
        ("🏃", "Mocap", vec![
            (NodeType::SplitSkeleton { picks: vec![SplitPick::Character(0), SplitPick::Character(1)] }, "One output per character in the take"),
            (NodeType::Characterize { picks: vec![] }, "Say which joint is which part of a human, for Body Collide and Retarget"),
            (NodeType::Retarget, "The motion of one skeleton on another"),
            (NodeType::AutoTPose { set_hip_height: false, hip_height: 90.0 }, "A neutral pose, one frame"),
            (NodeType::FixPose { edits: vec![] }, "Correct joints by hand, on every frame"),
            (NodeType::ProxySkin { thickness: 1.0 }, "A body of spheres and cylinders, bound to the skeleton"),
            (NodeType::Ragdoll { settings: Default::default(), view: Default::default(), limits: Default::default() }, "Keep the character out of the set and out of itself, within what a human body can do"),
        ]),
    ]
}

/// Name for a new node of this type: its label without spaces.
pub fn default_name(t: &NodeType) -> String {
    node_type_label(t).chars().filter(|c| c.is_alphanumeric()).collect()
}

fn add_node_menu(ui: &mut egui::Ui, graph: &mut NodeGraphState, cp: egui::Pos2) -> bool {
    let mut added = false;
    ui.set_min_width(150.0);
    // Categories look like the items in them: menu rows, not buttons.
    ui.visuals_mut().button_frame = false;
    for (icon, category, nodes) in catalog() {
        ui.menu_button(format!("{icon}  {category}"), |ui| {
            ui.set_min_width(170.0);
            for (t, hint) in nodes {
                let label = format!("{}  {}", node_type_icon(&t), node_type_label(&t));
                if ui.button(label).on_hover_text(hint).clicked() {
                    let name = default_name(&t);
                    graph.add_node(name, t, cp);
                    added = true;
                }
            }
        });
    }
    if added { ui.close_menu(); }
    added
}

// ============================================================================
// BEZIER HELPERS
// ============================================================================

pub fn draw_wire(painter: &egui::Painter, from: egui::Pos2, to: egui::Pos2, hovered: bool, selected: bool) {
    let off = (to.y - from.y).abs().max(60.0) * 0.5;
    let c1  = egui::pos2(from.x, from.y + off);
    let c2  = egui::pos2(to.x,   to.y   - off);
    let pts: Vec<egui::Pos2> =
        (0..=20).map(|i| bezier(from, c1, c2, to, i as f32 / 20.0)).collect();
    let (width, color) = if selected {
        (3.5, egui::Color32::from_rgb(220, 120, 80))
    } else if hovered {
        (3.5, xsi::WIRE_HOV())
    } else {
        (2.0, xsi::WIRE())
    };
    painter.add(egui::Shape::line(pts, egui::Stroke::new(width, color)));
}

fn is_near_bezier(p: egui::Pos2, from: egui::Pos2, to: egui::Pos2, thresh: f32) -> bool {
    let off = (to.y - from.y).abs().max(60.0) * 0.5;
    let c1  = egui::pos2(from.x, from.y + off);
    let c2  = egui::pos2(to.x,   to.y   - off);
    (0..=20).any(|i| {
        let q = bezier(from, c1, c2, to, i as f32 / 20.0);
        ((q.x - p.x).powi(2) + (q.y - p.y).powi(2)).sqrt() < thresh
    })
}

fn bezier(p0: egui::Pos2, p1: egui::Pos2, p2: egui::Pos2, p3: egui::Pos2, t: f32) -> egui::Pos2 {
    let mt = 1.0 - t;
    egui::pos2(
        mt*mt*mt*p0.x + 3.0*mt*mt*t*p1.x + 3.0*mt*t*t*p2.x + t*t*t*p3.x,
        mt*mt*mt*p0.y + 3.0*mt*mt*t*p1.y + 3.0*mt*t*t*p2.y + t*t*t*p3.y,
    )
}