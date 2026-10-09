use bevy::prelude::Vec3;
use bevy_egui::egui;
use std::collections::HashSet;
use crate::types::{ConnectionId, NodeId, SubnetNodeType};
use super::{SubnetGraph, SubnetNode};

// ── ICE look, named ───────────────────────────────────────────────────────────
// Modelled on Softimage ICE: nodes in solid colours by kind (green for data
// and maths, blue for getting and setting geometry, teal for the tree's in and
// out) with a two-line title (the node's own name in bold, its type beneath)
// and a collapse box; a row per port,
// outputs first then inputs; small dots on the node's edge coloured by data
// type, wires in the same colour; a light grey canvas with a fine grid. Every
// size and colour is here, so the look can be tuned in one place.

const TITLE_H:  f32 = 32.0;   // two lines: the node's name, then its type
const FOLDED_H: f32 = 20.0;   // a collapsed node: its name only
const ROW_H:    f32 = 15.0;   // one port per row
const PAD_B:    f32 = 4.0;    // space under the last row
const MIN_W:    f32 = 96.0;
const TITLE_CH: f32 = 7.4;    // width of a title character, for sizing nodes
const LABEL_CH: f32 = 5.9;    // width of a label character
const ICONS_W:  f32 = 20.0;   // collapse box, right of the title
const PORT_R:   f32 = 3.2;
const SOCK_HIT: f32 = 16.0;   // invisible hit area around a port
const ROUNDING: f32 = 5.0;
const TYPE_PT:  f32 = 9.5;    // the type line under the name
const TITLE_PT: f32 = 12.5;
const LABEL_PT: f32 = 10.5;
const GRID:     f32 = 12.0;   // canvas grid spacing
const WIRE_W:   f32 = 1.6;

mod pal {
    #![allow(non_snake_case)]
    use bevy_egui::egui::Color32;
    // ICE's own look, the same in both themes.
    pub fn BG() -> Color32 { Color32::from_rgb(170, 170, 170) }
    pub fn GRID() -> Color32 { Color32::from_rgb(161, 161, 161) }
    pub fn BREADCRUMB_BG() -> Color32 { crate::theme::c( 78,  78,  82) }
    pub fn TEXT() -> Color32 { crate::theme::c(248, 248, 248) }
    pub fn TEXT_DIM() -> Color32 { crate::theme::c(210, 210, 210) }

    pub fn INK() -> Color32 { Color32::from_rgb( 12,  12,  12) }       // titles and labels
    pub fn INK_TYPE() -> Color32 { Color32::from_rgba_unmultiplied(12, 12, 12, 150) }  // the type line
    pub fn DIVIDER() -> Color32 { Color32::from_black_alpha(80) }     // the line under the title
    pub fn RIM() -> Color32 { Color32::from_rgb( 38,  38,  38) }       // the node's edge
    pub fn FRAME() -> Color32 { Color32::from_rgb(105, 105, 105) }     // the grey ring around it
    pub fn SELECTED() -> Color32 { Color32::from_rgb(255, 255, 255) }
    pub fn PORT_RIM() -> Color32 { Color32::from_rgb( 30,  30,  30) }

    // Node colours, by kind.
    pub fn GREEN() -> Color32 { Color32::from_rgb(152, 196,  98) }     // data, constants, maths, generators
    pub fn BLUE() -> Color32  { Color32::from_rgb(102, 150, 204) }     // get / set geometry
    pub fn TEAL() -> Color32  { Color32::from_rgb(112, 186, 154) }     // the tree's in and out

    // Data types, as ICE colours their ports and wires.
    pub fn SCALAR() -> Color32  { Color32::from_rgb( 70, 230,  70) }   // Float: bright green
    pub fn INTEGER() -> Color32 { Color32::from_rgb( 20, 120,  40) }   // Int: dark green
    pub fn VECTOR() -> Color32  { Color32::from_rgb(235, 225,  60) }   // Vec3: yellow
    pub fn GEO() -> Color32     { Color32::from_rgb(220,  40, 200) }   // Mesh / geometry: magenta
    pub fn OTHER() -> Color32   { Color32::from_rgb( 85,  85,  85) }   // execute and the rest: dark grey
}

/// Where everything of a node sits, relative to its top-left corner.
struct Layout {
    size:    egui::Vec2,
    inputs:  Vec<egui::Vec2>,
    outputs: Vec<egui::Vec2>,
}

/// The title: the node type's full name in PascalCase (`ScatterPoints`,
/// `CrossProduct`, `SubnetInput`).
fn title_text(node: &SubnetNode) -> String {
    crate::types::subnet_node_label(&node.node_type)
        .split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect()
}

/// A constant's value, shown in a row of its own under the title.
fn value_text(node: &SubnetNode) -> Option<String> {
    match &node.node_type {
        SubnetNodeType::ConstVec3  { value } => Some(format!("<{}, {}, {}>", short(value.x), short(value.y), short(value.z))),
        SubnetNodeType::ConstFloat { value } => Some(short(*value)),
        SubnetNodeType::ConstInt   { value } => Some(value.to_string()),
        _ => None,
    }
}

/// A number as short as it reads: 1, 0.5, 0.125.
fn short(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// Rows: the outputs, then the inputs. Collapsed, a node is its title row
/// alone, every port on its edge at mid-height.
fn layout(node: &SubnetNode, collapsed: bool) -> Layout {
    let title = (node.name.chars().count() as f32 * TITLE_CH).max(title_text(node).chars().count() as f32 * LABEL_CH) + ICONS_W + 18.0;
    let label = |n: &str| n.chars().count() as f32 * LABEL_CH + 22.0;
    let value = value_text(node);
    let widest = node.outputs.iter().map(|o| label(&o.name))
        .chain(node.inputs.iter().map(|i| label(&i.name)))
        .chain(value.as_deref().map(label))
        .fold(0.0, f32::max);
    let w = title.max(widest).max(MIN_W).round();
    if collapsed {
        let mid = FOLDED_H * 0.5;
        return Layout {
            size:    egui::vec2(w, FOLDED_H),
            inputs:  vec![egui::vec2(0.0, mid); node.inputs.len()],
            outputs: vec![egui::vec2(w, mid); node.outputs.len()],
        };
    }
    // A constant's value takes the first row.
    let first = value.is_some() as usize;
    let row_y = |r: usize| TITLE_H + (r as f32 + 0.5) * ROW_H;
    let outs = node.outputs.len();
    Layout {
        size:    egui::vec2(w, TITLE_H + (first + outs + node.inputs.len()) as f32 * ROW_H + PAD_B),
        outputs: (0..outs).map(|r| egui::vec2(w, row_y(first + r))).collect(),
        inputs:  (0..node.inputs.len()).map(|r| egui::vec2(0.0, row_y(first + outs + r))).collect(),
    }
}

fn input_socket_pos(node: &SubnetNode, idx: usize, collapsed: &HashSet<NodeId>) -> egui::Pos2 {
    node.position + layout(node, collapsed.contains(&node.id)).inputs.get(idx).copied().unwrap_or_default()
}

fn output_socket_pos(node: &SubnetNode, idx: usize, collapsed: &HashSet<NodeId>) -> egui::Pos2 {
    node.position + layout(node, collapsed.contains(&node.id)).outputs.get(idx).copied().unwrap_or_default()
}

/// A node's colour, by kind.
fn tint(t: &SubnetNodeType) -> egui::Color32 {
    match t {
        SubnetNodeType::SubInput | SubnetNodeType::SubOutput => pal::TEAL(),
        SubnetNodeType::GetTemplate | SubnetNodeType::CopyToPoints => pal::BLUE(),
        _ => pal::GREEN(),
    }
}

/// Which nodes are collapsed: view state, kept with the canvas, not saved.
fn collapsed_id() -> egui::Id { egui::Id::new("ice_collapsed_nodes") }

// ── Breadcrumb ────────────────────────────────────────────────────────────────

pub fn draw_breadcrumb(ui: &mut egui::Ui, subnet_name: &str) -> bool {
    let mut exit = false;
    ui.horizontal(|ui| {
        egui::Frame::none()
            .fill(pal::BREADCRUMB_BG())
            .inner_margin(egui::vec2(8.0, 4.0))
            .show(ui, |ui| {
                if ui.link(egui::RichText::new("Root").color(pal::TEXT())).clicked() { exit = true; }
                ui.label(egui::RichText::new(" › ").color(pal::TEXT_DIM()));
                ui.label(egui::RichText::new(subnet_name)
                    .color(egui::Color32::from_rgb(180, 200, 230))
                    .strong());
            });
    });
    exit
}

// ── Subnet node properties ────────────────────────────────────────────────────

/// The parameters of a node inside an ICE subnet, laid out like every
/// other node's (see `properties::form`).
pub fn draw_subnet_node_properties(ui: &mut egui::Ui, graph: &mut SubnetGraph) {
    use crate::properties::form::{group, Status};
    let Some(node) = graph.selected_node.and_then(|s| graph.nodes.iter_mut().find(|n| n.id == s)) else {
        ui.label("Select a node to see its parameters.");
        return;
    };
    let title = format!("{}  {}", crate::types::subnet_node_icon(&node.node_type), crate::types::subnet_node_label(&node.node_type));
    group(ui, &title, None, |f| { f.text("Name", None, &mut node.name, ""); });
    match &mut node.node_type {
        SubnetNodeType::ConstVec3 { value } => group(ui, "Value", None, |f| {
            let mut v = value.to_array();
            if f.xyz("Value", None, &mut v, 0.01, "") { *value = bevy::math::Vec3::from_array(v); }
        }),
        SubnetNodeType::ConstFloat { value } => group(ui, "Value", None, |f| { f.drag("Value", None, value, 0.01, ""); }),
        SubnetNodeType::ConstInt { value } => group(ui, "Value", None, |f| {
            f.row("Value", None, |ui| ui.add(egui::DragValue::new(value).speed(1)));
        }),
        SubnetNodeType::MultiplyVec3 { scalar } => group(ui, "Multiply", None, |f| { f.drag("Scalar", None, scalar, 0.01, ""); }),
        SubnetNodeType::LerpVec3 { t } => group(ui, "Blend", None, |f| { f.slider("Amount", None, t, 0.0..=1.0, ""); }),
        SubnetNodeType::ScatterPoints { count, seed } => group(ui, "Points", None, |f| {
            f.slider_u32("Count", None, count, 1..=10_000, "");
            f.row("Seed", None, |ui| ui.add(egui::DragValue::new(seed).speed(1)));
        }),
        _ => group(ui, "Parameters", None, |f| f.status(Status::Info, "None")),
    }
}

// ── Main canvas ───────────────────────────────────────────────────────────────

pub fn draw_subnet_graph(ui: &mut egui::Ui, graph: &mut SubnetGraph) {
    let (response, painter) = ui.allocate_painter(
        egui::Vec2::new(ui.available_width(), ui.available_height()),
        egui::Sense::click_and_drag(),
    );
    let canvas_rect = response.rect;
    let pan = graph.pan_offset;
    let to_screen = |p: egui::Pos2| canvas_rect.min + p.to_vec2() + pan;

    // Light grey canvas with a fine grid that moves with the view, as ICE's.
    painter.rect_filled(canvas_rect, 0.0, pal::BG());
    let grid = egui::Stroke::new(1.0_f32, pal::GRID());
    let mut x = canvas_rect.min.x + pan.x.rem_euclid(GRID);
    while x < canvas_rect.max.x {
        painter.line_segment([egui::pos2(x, canvas_rect.min.y), egui::pos2(x, canvas_rect.max.y)], grid);
        x += GRID;
    }
    let mut y = canvas_rect.min.y + pan.y.rem_euclid(GRID);
    while y < canvas_rect.max.y {
        painter.line_segment([egui::pos2(canvas_rect.min.x, y), egui::pos2(canvas_rect.max.x, y)], grid);
        y += GRID;
    }
    let mut collapsed: HashSet<NodeId> = ui.data(|d| d.get_temp(collapsed_id())).unwrap_or_default();

    // Existing connections
    let mut hovered: Option<ConnectionId> = None;
    for conn in &graph.connections.clone() {
        if let (Some(fn_), Some(tn)) = (
            graph.nodes.iter().find(|n| n.id == conn.from_node),
            graph.nodes.iter().find(|n| n.id == conn.to_node),
        ) {
            let fp = to_screen(output_socket_pos(fn_, conn.from_output, &collapsed));
            let tp = to_screen(input_socket_pos(tn,   conn.to_input, &collapsed));
            if let Some(ptr) = response.hover_pos() {
                if is_near_bezier_h(ptr, fp, tp, 10.0) { hovered = Some(conn.id); }
            }
            let colour = type_colour(fn_.outputs.get(conn.from_output).map(|o| o.value_hint).unwrap_or(""));
            draw_wire_h(&painter, fp, tp, colour, hovered == Some(conn.id));
        }
    }
    if let Some(cid) = hovered {
        if response.secondary_clicked() { graph.remove_connection(cid); }
    }

    // In-progress wire — use raw pointer pos, not response.hover_pos()
    if let Some((fid, fo)) = graph.connecting_from {
        if let Some(fn_) = graph.nodes.iter().find(|n| n.id == fid) {
            let fp  = to_screen(output_socket_pos(fn_, fo, &collapsed));
            let ptr = ui.input(|i| i.pointer.hover_pos());
            let colour = type_colour(fn_.outputs.get(fo).map(|o| o.value_hint).unwrap_or(""));
            if let Some(ptr) = ptr { draw_wire_h(&painter, fp, ptr, colour, false); }
        }
    }

    // Cancel on Escape
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) { graph.connecting_from = None; }

    // Draw all nodes — sockets handle their own press/release via raw input
    let nodes_clone = graph.nodes.clone();
    for node in &nodes_clone {
        draw_subnet_node(ui, &painter, graph, node, &to_screen, canvas_rect, pan, &mut collapsed);
    }
    ui.data_mut(|d| d.insert_temp(collapsed_id(), collapsed));

    // Cancel wire only if mouse released and no socket consumed it
    // (drag_stopped on the background canvas means we missed all sockets)
    if graph.connecting_from.is_some() && response.drag_stopped() {
        graph.connecting_from = None;
    }

    // Pan with Shift+drag or MMB
    let is_mmb = ui.input(|i| i.pointer.middle_down());
    if response.dragged() && (ui.input(|i| i.modifiers.shift) || is_mmb) {
        graph.pan_offset += response.drag_delta();
    }

    // Context menu
    response.context_menu(|ui| {
        let ptr = ui.input(|i| i.pointer.hover_pos().unwrap_or_default());
        let cp  = ptr - canvas_rect.min.to_vec2() - graph.pan_offset;

        ui.label(egui::RichText::new("Constants").strong());
        if ui.button("→v  Const Vec3").clicked() {
            graph.add_node("Vec3".into(), SubnetNodeType::ConstVec3 { value: Vec3::ZERO }, cp);
            ui.close_menu();
        }
        if ui.button("→f  Const Float").clicked() {
            graph.add_node("Float".into(), SubnetNodeType::ConstFloat { value: 0.0 }, cp);
            ui.close_menu();
        }
        if ui.button("→i  Const Int").clicked() {
            graph.add_node("Int".into(), SubnetNodeType::ConstInt { value: 0 }, cp);
            ui.close_menu();
        }
        ui.separator();
        ui.label(egui::RichText::new("Vec3 Math").strong());
        if ui.button("+  Add").clicked() {
            graph.add_node("Add".into(), SubnetNodeType::AddVec3, cp);
            ui.close_menu();
        }
        if ui.button("-  Subtract").clicked() {
            graph.add_node("Subtract".into(), SubnetNodeType::SubtractVec3, cp);
            ui.close_menu();
        }
        if ui.button("×  Multiply").clicked() {
            graph.add_node("Multiply".into(), SubnetNodeType::MultiplyVec3 { scalar: 1.0 }, cp);
            ui.close_menu();
        }
        if ui.button("×  Cross Product").clicked() {
            graph.add_node("Cross".into(), SubnetNodeType::CrossProduct, cp);
            ui.close_menu();
        }
        if ui.button("|v|  Normalize").clicked() {
            graph.add_node("Normalize".into(), SubnetNodeType::Normalize, cp);
            ui.close_menu();
        }
        ui.separator();
        ui.label(egui::RichText::new("Scalar").strong());
        if ui.button("·  Dot Product").clicked() {
            graph.add_node("Dot".into(), SubnetNodeType::DotProduct, cp);
            ui.close_menu();
        }
        ui.separator();
        ui.label(egui::RichText::new("Interpolate").strong());
        if ui.button("≈  Lerp").clicked() {
            graph.add_node("Lerp".into(), SubnetNodeType::LerpVec3 { t: 0.5 }, cp);
            ui.close_menu();
        }
        ui.separator();
        ui.label(egui::RichText::new("Points").strong());
        if ui.button("→  Scatter Points").clicked() {
            graph.add_node("Scatter".into(), SubnetNodeType::ScatterPoints { count: 100, seed: 0 }, cp);
            ui.close_menu();
        }
        ui.separator();
        ui.label(egui::RichText::new("Geometry").strong());
        if ui.button("📦  Get Template").clicked() {
            graph.add_node("GetTemplate".into(), SubnetNodeType::GetTemplate, cp);
            ui.close_menu();
        }
        if ui.button("❇  Copy to Points").clicked() {
            graph.add_node("CopyToPoints".into(), SubnetNodeType::CopyToPoints, cp);
            ui.close_menu();
        }
    });
}

// ── Single node ───────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_subnet_node(
    ui:          &mut egui::Ui,
    painter:     &egui::Painter,
    graph:       &mut SubnetGraph,
    node:        &SubnetNode,
    to_screen:   &impl Fn(egui::Pos2) -> egui::Pos2,
    canvas_rect: egui::Rect,
    pan_offset:  egui::Vec2,
    collapsed:   &mut HashSet<NodeId>,
) {
    let id      = node.id;
    let folded  = collapsed.contains(&id);
    let np      = to_screen(node.position);
    let lay     = layout(node, folded);
    let rect    = egui::Rect::from_min_size(np, lay.size);
    let is_sel  = graph.selected_node == Some(id);
    let body    = tint(&node.node_type);

    // A grey ring, the body, a dark edge; white around it when selected.
    let ring = rect.expand(2.0);
    painter.rect_filled(ring, if ROUNDING > 0.0 { ROUNDING + 2.0 } else { 0.0 }, if is_sel { pal::SELECTED() } else { pal::FRAME() });
    painter.rect_filled(rect, ROUNDING, body);
    painter.rect_stroke(rect, ROUNDING, egui::Stroke::new(1.0_f32, pal::RIM()));

    // Two-line title: the node's own name in bold (drawn twice, a hair
    // apart), its type in small dim text beneath. Collapsed, only the name.
    let name_y = if folded { np.y + FOLDED_H * 0.5 } else { np.y + 12.0 };
    for dx in [0.0, 0.6] {
        painter.text(egui::pos2(np.x + 7.0 + dx, name_y), egui::Align2::LEFT_CENTER, &node.name,
            egui::FontId::proportional(TITLE_PT), pal::INK());
    }
    if !folded {
        painter.text(egui::pos2(np.x + 7.0, np.y + 25.0), egui::Align2::LEFT_CENTER, title_text(node),
            egui::FontId::proportional(TYPE_PT), pal::INK_TYPE());
        // A thin line between the title and the ports, short of the edges.
        let y = np.y + TITLE_H - 0.5;
        painter.line_segment([egui::pos2(rect.min.x + 4.0, y), egui::pos2(rect.max.x - 4.0, y)],
            egui::Stroke::new(1.0_f32, pal::DIVIDER()));
    }

    // A constant's value, under the title.
    if let (false, Some(v)) = (folded, value_text(node)) {
        painter.text(egui::pos2(np.x + 7.0, np.y + TITLE_H + ROW_H * 0.5), egui::Align2::LEFT_CENTER, v,
            egui::FontId::monospace(LABEL_PT), pal::INK());
    }

    // The collapse box, at the right of the title.
    let fold_rect = egui::Rect::from_center_size(egui::pos2(rect.max.x - 11.0, np.y + if folded { FOLDED_H * 0.5 } else { 12.0 }), egui::vec2(11.0, 9.0));
    draw_fold_box(painter, fold_rect);

    // ── IMPORTANT: title bar allocated FIRST → lowest hit-test priority ───────
    let title_rect = egui::Rect::from_min_size(np, egui::vec2(lay.size.x, if folded { FOLDED_H } else { TITLE_H }));
    let dr = ui.allocate_rect(title_rect, egui::Sense::click_and_drag());
    let fold = ui.allocate_rect(fold_rect.expand(2.0), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if folded { "Expand" } else { "Collapse" });
    if fold.clicked() {
        if !collapsed.remove(&id) { collapsed.insert(id); }
    }

    // ── Output ports (right edge) — allocated before inputs, highest priority
    for (i, out) in node.outputs.iter().enumerate() {
        let sp  = np + lay.outputs[i];
        let hit = egui::Rect::from_center_size(sp, egui::vec2(SOCK_HIT, SOCK_HIT));
        let sr  = ui.allocate_rect(hit, egui::Sense::click_and_drag());
        let wiring = graph.connecting_from == Some((id, i));
        draw_port(painter, sp, type_colour(out.value_hint), sr.hovered() || wiring);
        if !folded {
            painter.text(egui::pos2(sp.x - 7.0, sp.y), egui::Align2::RIGHT_CENTER, &out.name,
                egui::FontId::proportional(LABEL_PT), pal::INK());
        }

        // Fire on the very first frame of press
        if sr.drag_started() || (sr.is_pointer_button_down_on() && graph.connecting_from.is_none()) {
            graph.connecting_from = Some((id, i));
        }
    }

    // ── Input ports (left edge) ───────────────────────────────────────────────
    for (i, inp) in node.inputs.iter().enumerate() {
        let sp  = np + lay.inputs[i];
        let hit = egui::Rect::from_center_size(sp, egui::vec2(SOCK_HIT, SOCK_HIT));
        let sr  = ui.allocate_rect(hit, egui::Sense::drag());
        // Hovered while a wire is being drawn: a target.
        let target = sr.hovered() && graph.connecting_from.is_some();
        draw_port(painter, sp, type_colour(inp.value_hint), target);
        if !folded {
            painter.text(egui::pos2(sp.x + 7.0, sp.y), egui::Align2::LEFT_CENTER, &inp.name,
                egui::FontId::proportional(LABEL_PT), pal::INK());
        }

        // Complete wire: mouse released while this socket is hovered.
        // Use raw input so the canvas response can't steal the release.
        if sr.hovered() && ui.input(|s| s.pointer.primary_released()) {
            if let Some((fn_, fo)) = graph.connecting_from {
                if fn_ != id { graph.add_connection(fn_, fo, id, i); }
                graph.connecting_from = None;
            }
        }
    }

    // ── Title bar interactions (lowest priority — allocated first) ────────────
    if dr.clicked() { graph.selected_node = Some(id); }
    // Only drag the node if we are not mid-wire
    if dr.drag_started() && graph.connecting_from.is_none() {
        graph.dragging_node = Some(id);
        graph.drag_offset   = dr.interact_pointer_pos().unwrap_or_default() - np;
    }
    if dr.dragged() && graph.dragging_node == Some(id) {
        if let Some(ptr) = dr.interact_pointer_pos() {
            let new_pos = ptr - pan_offset - canvas_rect.min.to_vec2() - graph.drag_offset;
            if let Some(n) = graph.nodes.iter_mut().find(|n| n.id == id) { n.position = new_pos; }
        }
    }
    if dr.drag_stopped() { graph.dragging_node = None; }
}

/// A port: a small dot in its type's colour on the node's edge, larger
/// under the pointer.
fn draw_port(painter: &egui::Painter, at: egui::Pos2, colour: egui::Color32, hot: bool) {
    let r = if hot { PORT_R + 1.5 } else { PORT_R };
    painter.circle_filled(at, r + 1.0, pal::PORT_RIM());
    painter.circle_filled(at, r, colour);
}

/// The collapse box: a small grey box with three lines in it.
fn draw_fold_box(painter: &egui::Painter, r: egui::Rect) {
    painter.rect_filled(r, 1.5, egui::Color32::from_rgb(200, 200, 200));
    painter.rect_stroke(r, 1.5, egui::Stroke::new(1.0_f32, pal::RIM()));
    for k in 1..=3 {
        let y = r.min.y + r.height() * k as f32 / 4.0;
        painter.line_segment([egui::pos2(r.min.x + 2.0, y), egui::pos2(r.max.x - 2.0, y)], egui::Stroke::new(1.0_f32, pal::RIM()));
    }
}

// ── Colour helpers ────────────────────────────────────────────────────────────

/// A data type's colour, as ICE shows it on ports and wires.
fn type_colour(hint: &str) -> egui::Color32 {
    match hint {
        "Float" => pal::SCALAR(),
        "Int"   => pal::INTEGER(),
        "Vec3"  => pal::VECTOR(),
        "Mesh"  => pal::GEO(),
        _       => pal::OTHER(),
    }
}

// ── Wire drawing ──────────────────────────────────────────────────────────────

/// A wire from an output to an input: a thin curve in the colour of the data
/// it carries.
pub fn draw_wire_h(painter: &egui::Painter, from: egui::Pos2, to: egui::Pos2, colour: egui::Color32, hovered: bool) {
    let off = (to.x - from.x).abs().max(60.0) * 0.5;
    let c1  = egui::pos2(from.x + off, from.y);
    let c2  = egui::pos2(to.x   - off, to.y);
    let pts: Vec<egui::Pos2> = (0..=24)
        .map(|i| bezier(from, c1, c2, to, i as f32 / 24.0))
        .collect();
    painter.add(egui::Shape::line(pts, egui::Stroke::new(
        if hovered { WIRE_W + 1.4 } else { WIRE_W },
        if hovered { egui::Color32::WHITE } else { colour })));
}

fn is_near_bezier_h(p: egui::Pos2, from: egui::Pos2, to: egui::Pos2, thresh: f32) -> bool {
    let off = (to.x - from.x).abs().max(60.0) * 0.5;
    let c1  = egui::pos2(from.x + off, from.y);
    let c2  = egui::pos2(to.x   - off, to.y);
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