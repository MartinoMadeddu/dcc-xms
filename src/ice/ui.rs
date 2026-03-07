use bevy::prelude::Vec3;
use bevy_egui::egui;
use crate::types::{ConnectionId, SubnetNodeType, subnet_node_icon};
use super::{SubnetGraph, SubnetNode};

const NODE_W:   f32 = 160.0;
const NODE_H:   f32 = 60.0;
const SOCK_R:   f32 = 5.0;
const SOCK_HIT: f32 = 22.0; // large invisible hit area
const ROUNDING: f32 = 8.0;

mod pal {
    use bevy_egui::egui::Color32;
    pub const BG:             Color32 = Color32::from_rgb( 95,  95,  95);
    pub const GRID:           Color32 = Color32::from_rgb( 85,  85,  85);
    pub const BODY_NORMAL:    Color32 = Color32::from_rgb(125, 125, 125);
    pub const BODY_TERMINAL:  Color32 = Color32::from_rgb(105, 118, 105);
    pub const BODY_CONST:     Color32 = Color32::from_rgb(115, 105, 122);
    pub const BODY_SEL:       Color32 = Color32::from_rgb(110, 120, 135);
    pub const TITLE_NORMAL:   Color32 = Color32::from_rgb(100, 100, 100);
    pub const TITLE_TERMINAL: Color32 = Color32::from_rgb( 82,  98,  82);
    pub const TITLE_CONST:    Color32 = Color32::from_rgb( 90,  78, 100);
    pub const BORDER:         Color32 = Color32::from_rgb( 68,  68,  68);
    pub const BORDER_SEL:     Color32 = Color32::from_rgb(175, 200, 220);
    pub const BORDER_TERM:    Color32 = Color32::from_rgb(120, 175, 120);
    pub const BORDER_CONST:   Color32 = Color32::from_rgb(170, 130, 200);
    pub const TEXT:           Color32 = Color32::from_rgb(230, 230, 230);
    pub const TEXT_DIM:       Color32 = Color32::from_rgb(185, 185, 185);
    pub const WIRE:           Color32 = Color32::from_rgb(160, 160, 160);
    pub const WIRE_HOV:       Color32 = Color32::from_rgb(220, 185,  90);
    pub const BREADCRUMB_BG:  Color32 = Color32::from_rgb( 78,  78,  82);
}

fn input_socket_pos(node: &SubnetNode, idx: usize) -> egui::Pos2 {
    let t = (idx + 1) as f32 / (node.inputs.len() + 1) as f32;
    egui::pos2(node.position.x, node.position.y + NODE_H * t)
}

fn output_socket_pos(node: &SubnetNode, idx: usize) -> egui::Pos2 {
    let t = (idx + 1) as f32 / (node.outputs.len() + 1) as f32;
    egui::pos2(node.position.x + NODE_W, node.position.y + NODE_H * t)
}

// ── Breadcrumb ────────────────────────────────────────────────────────────────

pub fn draw_breadcrumb(ui: &mut egui::Ui, subnet_name: &str) -> bool {
    let mut exit = false;
    ui.horizontal(|ui| {
        egui::Frame::none()
            .fill(pal::BREADCRUMB_BG)
            .inner_margin(egui::vec2(8.0, 4.0))
            .show(ui, |ui| {
                if ui.link(egui::RichText::new("Root").color(pal::TEXT)).clicked() { exit = true; }
                ui.label(egui::RichText::new(" › ").color(pal::TEXT_DIM));
                ui.label(egui::RichText::new(subnet_name)
                    .color(egui::Color32::from_rgb(180, 200, 230))
                    .strong());
            });
    });
    exit
}

// ── Subnet node properties ────────────────────────────────────────────────────

pub fn draw_subnet_node_properties(ui: &mut egui::Ui, graph: &mut SubnetGraph) {
    ui.heading("Properties");
    ui.separator();
    let sel = match graph.selected_node {
        Some(s) => s,
        None => { ui.label("No node selected."); return; }
    };
    let node = match graph.nodes.iter_mut().find(|n| n.id == sel) {
        Some(n) => n,
        None    => { ui.label("No node selected."); return; }
    };
    ui.label(format!("Node: {}", node.name));
    ui.separator();
    match &mut node.node_type {
        SubnetNodeType::ConstVec3 { value } => {
            ui.label("Constant Vec3");
            ui.horizontal(|ui| {
                ui.label("X:"); ui.add(egui::DragValue::new(&mut value.x).speed(0.01));
                ui.label("Y:"); ui.add(egui::DragValue::new(&mut value.y).speed(0.01));
                ui.label("Z:"); ui.add(egui::DragValue::new(&mut value.z).speed(0.01));
            });
        }
        SubnetNodeType::ConstFloat { value } => {
            ui.label("Constant Float");
            ui.add(egui::DragValue::new(value).speed(0.01));
        }
        SubnetNodeType::ConstInt { value } => {
            ui.label("Constant Integer");
            ui.add(egui::DragValue::new(value).speed(1));
        }
        SubnetNodeType::MultiplyVec3 { scalar } => {
            ui.label("Multiply Vec3");
            ui.horizontal(|ui| {
                ui.label("Scalar:");
                ui.add(egui::DragValue::new(scalar).speed(0.01));
            });
        }
        SubnetNodeType::LerpVec3 { t } => {
            ui.label("Lerp Vec3");
            ui.add(egui::Slider::new(t, 0.0..=1.0).text("t"));
        }
        SubnetNodeType::ScatterPoints { count, seed } => {
            ui.label("Scatter Points");
            ui.horizontal(|ui| {
                ui.label("Count:");
                ui.add(egui::DragValue::new(count).speed(1).clamp_range(1..=10000));
            });
            ui.horizontal(|ui| {
                ui.label("Seed:");
                ui.add(egui::DragValue::new(seed).speed(1));
            });
        }
        SubnetNodeType::SubInput  => { ui.label("Subnet Input — no parameters.");  }
        SubnetNodeType::SubOutput => { ui.label("Subnet Output — no parameters."); }
        _ => { ui.label("No editable parameters."); }

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

    // Background + grid
    painter.rect_filled(canvas_rect, 0.0, pal::BG);
    for x in (0..canvas_rect.width() as i32).step_by(50) {
        painter.line_segment(
            [egui::pos2(canvas_rect.min.x + x as f32, canvas_rect.min.y),
             egui::pos2(canvas_rect.min.x + x as f32, canvas_rect.max.y)],
            egui::Stroke::new(1.0, pal::GRID));
    }
    for y in (0..canvas_rect.height() as i32).step_by(50) {
        painter.line_segment(
            [egui::pos2(canvas_rect.min.x, canvas_rect.min.y + y as f32),
             egui::pos2(canvas_rect.max.x, canvas_rect.min.y + y as f32)],
            egui::Stroke::new(1.0, pal::GRID));
    }

    // Existing connections
    let mut hovered: Option<ConnectionId> = None;
    for conn in &graph.connections.clone() {
        if let (Some(fn_), Some(tn)) = (
            graph.nodes.iter().find(|n| n.id == conn.from_node),
            graph.nodes.iter().find(|n| n.id == conn.to_node),
        ) {
            let fp = to_screen(output_socket_pos(fn_, conn.from_output));
            let tp = to_screen(input_socket_pos(tn,   conn.to_input));
            if let Some(ptr) = response.hover_pos() {
                if is_near_bezier_h(ptr, fp, tp, 10.0) { hovered = Some(conn.id); }
            }
            draw_wire_h(&painter, fp, tp, hovered == Some(conn.id));
        }
    }
    if let Some(cid) = hovered {
        if response.secondary_clicked() { graph.remove_connection(cid); }
    }

    // In-progress wire — use raw pointer pos, not response.hover_pos()
    if let Some((fid, fo)) = graph.connecting_from {
        if let Some(fn_) = graph.nodes.iter().find(|n| n.id == fid) {
            let fp  = to_screen(output_socket_pos(fn_, fo));
            let ptr = ui.input(|i| i.pointer.hover_pos());
            if let Some(ptr) = ptr { draw_wire_h(&painter, fp, ptr, false); }
        }
    }

    // Cancel on Escape
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) { graph.connecting_from = None; }

    // Draw all nodes — sockets handle their own press/release via raw input
    let nodes_clone = graph.nodes.clone();
    for node in &nodes_clone {
        draw_subnet_node(ui, &painter, graph, node, &to_screen, canvas_rect, pan);
    }

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
        if ui.button("＋  Add").clicked() {
            graph.add_node("Add".into(), SubnetNodeType::AddVec3, cp);
            ui.close_menu();
        }
        if ui.button("－  Subtract").clicked() {
            graph.add_node("Subtract".into(), SubnetNodeType::SubtractVec3, cp);
            ui.close_menu();
        }
        if ui.button("✕  Multiply").clicked() {
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
    });
}

// ── Single node ───────────────────────────────────────────────────────────────

fn draw_subnet_node(
    ui:          &mut egui::Ui,
    painter:     &egui::Painter,
    graph:       &mut SubnetGraph,
    node:        &SubnetNode,
    to_screen:   &impl Fn(egui::Pos2) -> egui::Pos2,
    canvas_rect: egui::Rect,
    pan_offset:  egui::Vec2,
) {
    let np     = to_screen(node.position);
    let rect   = egui::Rect::from_min_size(np, egui::vec2(NODE_W, NODE_H));
    let id     = node.id;
    let is_sel = graph.selected_node == Some(id);

    let is_terminal = matches!(node.node_type, SubnetNodeType::SubInput | SubnetNodeType::SubOutput);
    let is_const    = matches!(node.node_type,
        SubnetNodeType::ConstVec3 { .. } | SubnetNodeType::ConstFloat { .. } | SubnetNodeType::ConstInt { .. });

    let body_color  = if is_terminal   { pal::BODY_TERMINAL }
                      else if is_const { pal::BODY_CONST    }
                      else if is_sel   { pal::BODY_SEL      }
                      else             { pal::BODY_NORMAL   };
    let title_color = if is_terminal   { pal::TITLE_TERMINAL }
                      else if is_const { pal::TITLE_CONST    }
                      else             { pal::TITLE_NORMAL   };
    let border_col  = if is_sel        { pal::BORDER_SEL   }
                      else if is_terminal { pal::BORDER_TERM }
                      else if is_const { pal::BORDER_CONST  }
                      else             { pal::BORDER        };

    // Draw body
    painter.rect_filled(rect, ROUNDING, body_color);
    painter.rect_stroke(rect, ROUNDING, egui::Stroke::new(
        if is_sel { 2.5 } else { 1.5 }, border_col));

    let title_rect = egui::Rect::from_min_size(np, egui::vec2(NODE_W, 30.0));
    painter.rect_filled(title_rect,
        egui::Rounding { nw: ROUNDING, ne: ROUNDING, sw: 0.0, se: 0.0 },
        title_color);
    painter.text(
        egui::pos2(np.x + NODE_W / 2.0, np.y + 15.0),
        egui::Align2::CENTER_CENTER,
        &format!("{} {}", subnet_node_icon(&node.node_type), node.name),
        egui::FontId::proportional(13.0), pal::TEXT);

    let hint = match &node.node_type {
        SubnetNodeType::ConstVec3  { value } =>
            format!("({:.2}, {:.2}, {:.2})", value.x, value.y, value.z),
        SubnetNodeType::ConstFloat { value } => format!("{:.3}", value),
        SubnetNodeType::ConstInt   { value } => format!("{}", value),
        _ => socket_type_label(&node.node_type).to_string(),
    };
    painter.text(
        egui::pos2(np.x + NODE_W / 2.0, np.y + NODE_H - 10.0),
        egui::Align2::CENTER_CENTER, &hint,
        egui::FontId::proportional(9.0), pal::TEXT_DIM);

    // ── IMPORTANT: title bar allocated FIRST → lowest hit-test priority ───────
    let dr = ui.allocate_rect(title_rect, egui::Sense::click_and_drag());

    // ── Output sockets (right side) — allocated before inputs, highest priority
    for (i, out) in node.outputs.iter().enumerate() {
        let t   = (i + 1) as f32 / (node.outputs.len() + 1) as f32;
        let sp  = egui::pos2(np.x + NODE_W, np.y + NODE_H * t);
        let hit = egui::Rect::from_center_size(sp, egui::vec2(SOCK_HIT, SOCK_HIT));
        let sr  = ui.allocate_rect(hit, egui::Sense::click_and_drag());

        let is_wiring = graph.connecting_from == Some((id, i));
        painter.circle_filled(sp, SOCK_R,
            socket_color_out(out.value_hint, sr.hovered(), is_wiring));
        painter.text(egui::pos2(sp.x - SOCK_R - 3.0, sp.y),
            egui::Align2::RIGHT_CENTER, &out.name,
            egui::FontId::proportional(10.0), pal::TEXT_DIM);

        // Fire on the very first frame of press
        if sr.drag_started() || (sr.is_pointer_button_down_on() && graph.connecting_from.is_none()) {
            graph.connecting_from = Some((id, i));
        }
    }

    // ── Input sockets (left side) ─────────────────────────────────────────────
    for (i, inp) in node.inputs.iter().enumerate() {
        let t   = (i + 1) as f32 / (node.inputs.len() + 1) as f32;
        let sp  = egui::pos2(np.x, np.y + NODE_H * t);
        let hit = egui::Rect::from_center_size(sp, egui::vec2(SOCK_HIT, SOCK_HIT));
        let sr  = ui.allocate_rect(hit, egui::Sense::drag());

        painter.circle_filled(sp, SOCK_R,
            socket_color(inp.value_hint, sr.hovered(), inp.connected_output.is_some()));
        painter.text(egui::pos2(sp.x + SOCK_R + 3.0, sp.y),
            egui::Align2::LEFT_CENTER, &inp.name,
            egui::FontId::proportional(10.0), pal::TEXT_DIM);

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

// ── Colour helpers ────────────────────────────────────────────────────────────

fn socket_color(hint: &str, hovered: bool, connected: bool) -> egui::Color32 {
    if hovered { return egui::Color32::from_rgb(220, 220, 220); }
    match hint {
        "Mesh"  => if connected { egui::Color32::from_rgb(210, 155,  60) } else { egui::Color32::from_rgb(155, 105, 40) },
        "Vec3"  => if connected { egui::Color32::from_rgb( 80, 175, 240) } else { egui::Color32::from_rgb( 50, 110,155) },
        "Float" => if connected { egui::Color32::from_rgb(150, 240, 110) } else { egui::Color32::from_rgb( 90, 145, 70) },
        "Int"   => if connected { egui::Color32::from_rgb(245, 195,  80) } else { egui::Color32::from_rgb(155, 115, 40) },
        _       => egui::Color32::from_gray(130),
    }
}

fn socket_color_out(hint: &str, hovered: bool, dragging: bool) -> egui::Color32 {
    if dragging { return egui::Color32::from_rgb(230, 195, 100); }
    socket_color(hint, hovered, false)
}

fn socket_type_label(t: &SubnetNodeType) -> &'static str {
    match t {
        SubnetNodeType::DotProduct          => "Vec3 → Float",
        SubnetNodeType::MultiplyVec3 { .. } => "Vec3 × scalar",
        SubnetNodeType::LerpVec3 { .. }     => "Vec3 lerp Vec3",
        SubnetNodeType::SubInput            => "Mesh in",
        SubnetNodeType::SubOutput           => "Mesh out",
        _                                   => "Vec3 → Vec3",
    }
}

// ── Wire drawing ──────────────────────────────────────────────────────────────

pub fn draw_wire_h(painter: &egui::Painter, from: egui::Pos2, to: egui::Pos2, hovered: bool) {
    let off = (to.x - from.x).abs().max(60.0) * 0.5;
    let c1  = egui::pos2(from.x + off, from.y);
    let c2  = egui::pos2(to.x   - off, to.y);
    let pts: Vec<egui::Pos2> = (0..=20)
        .map(|i| bezier(from, c1, c2, to, i as f32 / 20.0))
        .collect();
    painter.add(egui::Shape::line(pts, egui::Stroke::new(
        if hovered { 4.0 } else { 2.5 },
        if hovered { pal::WIRE_HOV } else { pal::WIRE })));
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