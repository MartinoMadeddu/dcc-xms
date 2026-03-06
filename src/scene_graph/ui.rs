use bevy_egui::egui;
use crate::node_graph::NodeGraphState;
use crate::types::{NodeId, SceneHierarchy, SceneObjectId};
use super::OperatorStack;

mod xsi {
    use bevy_egui::egui::Color32;
    pub const PANEL_BG:    Color32 = Color32::from_rgb(118, 118, 118);
    pub const HEADER:      Color32 = Color32::from_rgb(230, 230, 230);
    pub const NAME:        Color32 = Color32::from_rgb(220, 220, 220);
    pub const TYPE_LABEL:  Color32 = Color32::from_rgb(170, 170, 170);
    pub const SEL_BG:      Color32 = Color32::from_rgb( 90, 105, 120);
    pub const SEL_NAME:    Color32 = Color32::from_rgb(240, 238, 230);
    pub const DIVIDER:     Color32 = Color32::from_rgb( 90,  90,  90);
}

// ── Scene Explorer (object-centric) ──────────────────────────────────────────

const ROW_H:        f32 = 20.0;
const INDENT:       f32 = 16.0;
const EXPANDER_W:   f32 = 14.0;
const TREE_LINE_X:  f32 =  7.0;

pub fn draw_scene_explorer(
    ui:        &mut egui::Ui,
    hierarchy: &mut SceneHierarchy,
    graph:     &mut NodeGraphState,
) {
    egui::Frame::none()
        .fill(xsi::PANEL_BG)
        .inner_margin(6.0)
        .show(ui, |ui| {
            ui.colored_label(xsi::HEADER,
                egui::RichText::new("Scene Explorer").strong().size(14.0));
            ui.separator();

            egui::ScrollArea::vertical()
                .id_source("scene_explorer_scroll")
                .show(ui, |ui| {
                    let mut toggle_id = None;
                    let mut select_id: Option<(SceneObjectId, NodeId, Option<String>)> = None;

                    let n = hierarchy.objects.len();
                    let mut last_at_depth: Vec<bool> = vec![false; 16];
                    let mut collapsed_at_depth: Option<usize> = None;

                    for idx in 0..n {
                        let obj   = &hierarchy.objects[idx];
                        let depth = obj.depth;

                        // Skip rows that are inside a collapsed subtree
                        if let Some(cd) = collapsed_at_depth {
                            if depth > cd {
                                continue;
                            } else {
                                collapsed_at_depth = None;
                            }
                        }

                        // Is this the last item at its depth among its siblings?
                        let is_last = hierarchy.objects[idx + 1..]
                            .iter()
                            .find(|o| o.depth <= depth)
                            .map(|o| o.depth < depth)
                            .unwrap_or(true);
                        if depth < 16 { last_at_depth[depth] = is_last; }

                        let is_sel = hierarchy.selected.map(|s| s == obj.id).unwrap_or(false);

                        // Allocate a full-width row
                        let (row_rect, row_resp) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), ROW_H),
                            egui::Sense::click(),
                        );
                        if row_resp.clicked() {
                            select_id = Some((obj.id, obj.node_id, obj.prim_path.clone()));
                        }

                        let line_col  = egui::Color32::from_rgb(150, 150, 150);
                        let stroke    = egui::Stroke::new(1.0, line_col);
                        let indent_x  = row_rect.min.x + depth as f32 * INDENT;
                        let mid_y     = row_rect.center().y;
                        let exp_rect  = egui::Rect::from_center_size(
                            egui::pos2(indent_x + EXPANDER_W * 0.5, mid_y),
                            egui::vec2(EXPANDER_W, EXPANDER_W),
                        );

                        // Handle expander click BEFORE borrowing the painter
                        if obj.has_children {
                            if ui.allocate_rect(exp_rect, egui::Sense::click())
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                toggle_id = Some(obj.id);
                            }
                        }

                        // Now safe to take the painter
                        let painter = ui.painter();

                        // Selection background
                        if is_sel {
                            painter.rect_filled(row_rect, 0.0, xsi::SEL_BG);
                        }

                        // ── Tree lines ────────────────────────────────────────

                        for d in 0..depth {
                            if d < 16 && !last_at_depth[d] {
                                let x = row_rect.min.x + d as f32 * INDENT + TREE_LINE_X;
                                painter.line_segment(
                                    [egui::pos2(x, row_rect.min.y),
                                     egui::pos2(x, row_rect.max.y)],
                                    stroke,
                                );
                            }
                        }

                        if depth > 0 {
                            let x     = row_rect.min.x + (depth - 1) as f32 * INDENT + TREE_LINE_X;
                            let bot_y = if is_last { mid_y } else { row_rect.max.y };
                            painter.line_segment(
                                [egui::pos2(x, row_rect.min.y), egui::pos2(x, bot_y)],
                                stroke,
                            );
                            let elbow_end = row_rect.min.x + depth as f32 * INDENT;
                            painter.line_segment(
                                [egui::pos2(x, mid_y), egui::pos2(elbow_end, mid_y)],
                                stroke,
                            );
                        }

                        // ── +/- expander box ──────────────────────────────────
                        if obj.has_children {
                            painter.rect_stroke(exp_rect, 1.0,
                                egui::Stroke::new(1.0, line_col));
                            let cx = exp_rect.center().x;
                            let cy = exp_rect.center().y;
                            painter.line_segment(
                                [egui::pos2(cx - 3.0, cy), egui::pos2(cx + 3.0, cy)],
                                egui::Stroke::new(1.5, xsi::NAME),
                            );
                            if !obj.expanded {
                                painter.line_segment(
                                    [egui::pos2(cx, cy - 3.0), egui::pos2(cx, cy + 3.0)],
                                    egui::Stroke::new(1.5, xsi::NAME),
                                );
                            }
                        }

                        // ── Icon + label ──────────────────────────────────────
                        let text_x   = indent_x + EXPANDER_W + 4.0;
                        let text_col = if is_sel { xsi::SEL_NAME } else { xsi::NAME };
                        painter.text(
                            egui::pos2(text_x, mid_y),
                            egui::Align2::LEFT_CENTER,
                            format!("{} {}", obj.icon, obj.name),
                            egui::FontId::proportional(12.0),
                            text_col,
                        );

                        // Track collapsed subtrees AFTER rendering this row
                        if obj.has_children && !obj.expanded {
                            collapsed_at_depth = Some(depth);
                        }
                    }

                    if let Some(id) = toggle_id {
                        if let Some(obj) = hierarchy.objects.iter_mut().find(|o| o.id == id) {
                            obj.expanded = !obj.expanded;
                        }
                    }
                    if let Some((scene_id, node_id, prim_path)) = select_id {
                        hierarchy.selected           = Some(scene_id);
                        hierarchy.selected_prim_path = prim_path;
                        graph.selected_node          = Some(node_id);
                    }
                });
        });
}

// ── Operator Stack (node-graph mirror) ───────────────────────────────────────

pub fn draw_operator_stack(
    ui:    &mut egui::Ui,
    stack: &mut OperatorStack,
    graph: &mut NodeGraphState,
) {
    egui::Frame::none()
        .fill(xsi::PANEL_BG)
        .inner_margin(6.0)
        .show(ui, |ui| {
            ui.colored_label(xsi::HEADER,
                egui::RichText::new("Operator Stack").strong().size(14.0));
            ui.separator();

            egui::ScrollArea::vertical()
                .id_source("op_stack_scroll")
                .show(ui, |ui| {
                    let mut toggle_id = None;
                    let mut select_id = None;

                    for entry in &stack.entries {
                        let indent = entry.depth as f32 * 16.0;
                        let is_sel = stack.selected_entry == Some(entry.node_id)
                            || (stack.selected_entry.is_none()
                                && graph.selected_node == Some(entry.node_id));

                        ui.horizontal(|ui| {
                            ui.add_space(indent);

                            if entry.has_children {
                                let tri = if entry.expanded { "▾" } else { "▸" };
                                if ui.small_button(tri).clicked() {
                                    toggle_id = Some(entry.node_id);
                                }
                            } else {
                                ui.add_space(18.0);
                            }

                            let btn = egui::Button::new(
                                egui::RichText::new(
                                    format!("{} {}", entry.type_icon, entry.name)
                                )
                                .color(if is_sel { xsi::SEL_NAME } else { xsi::NAME })
                            )
                            .fill(if is_sel { xsi::SEL_BG } else { egui::Color32::TRANSPARENT })
                            .frame(true)
                            .min_size(egui::vec2(ui.available_width(), 36.0));

                            let resp = ui.add(btn);

                            ui.painter().text(
                                egui::pos2(resp.rect.min.x + indent + 22.0, resp.rect.min.y + 20.0),
                                egui::Align2::LEFT_TOP,
                                entry.type_label,
                                egui::FontId::proportional(9.0),
                                xsi::TYPE_LABEL,
                            );

                            if resp.clicked() { select_id = Some(entry.node_id); }
                        });
                    }

                    if let Some(id) = toggle_id {
                        if let Some(e) = stack.entries.iter_mut().find(|e| e.node_id == id) {
                            e.expanded = !e.expanded;
                        }
                        stack.rebuild(&graph.nodes, &graph.connections);
                    }
                    if let Some(id) = select_id {
                        stack.selected_entry = Some(id);
                        graph.selected_node  = Some(id);
                    }
                });
        });
}