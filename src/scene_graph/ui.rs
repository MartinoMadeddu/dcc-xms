use bevy_egui::egui;
use crate::node_graph::NodeGraphState;
use crate::types::{NodeId, SceneHierarchy, SceneObjectId};
use super::OperatorStack;

mod xsi {
    // Light-theme colours. `theme::c` returns the dark counterpart in dark mode.
    #![allow(non_snake_case)]
    use bevy_egui::egui::Color32;
    pub fn PANEL_BG() -> Color32 { crate::theme::c(118, 118, 118) }
    pub fn HEADER() -> Color32 { crate::theme::c(248, 248, 248) }
    pub fn NAME() -> Color32 { crate::theme::c(238, 238, 238) }
    pub fn TYPE_LABEL() -> Color32 { crate::theme::c(210, 210, 210) }
    pub fn SEL_BG() -> Color32 { crate::theme::c( 90, 105, 120) }
    pub fn SEL_NAME() -> Color32 { crate::theme::c(240, 238, 230) }
    pub fn HOVER_BG() -> Color32 { crate::theme::c(108, 108, 108) }
    pub fn DIVIDER() -> Color32 { crate::theme::c( 90,  90,  90) }
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
        .fill(xsi::PANEL_BG())
        .inner_margin(6.0)
        .show(ui, |ui| {
            ui.colored_label(xsi::HEADER(),
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

                        let line_col  = xsi::DIVIDER();
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
                            painter.rect_filled(row_rect, 0.0, xsi::SEL_BG());
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

                        // ── Expander: a chevron, down when open ────────────────
                        if obj.has_children {
                            chevron(painter, exp_rect.center(), obj.expanded, xsi::TYPE_LABEL());
                        }

                        // ── Icon + label ──────────────────────────────────────
                        let text_x   = indent_x + EXPANDER_W + 4.0;
                        let text_col = if is_sel { xsi::SEL_NAME() } else { xsi::NAME() };
                        // Joints get a drawn dot: filled when the joint has children.
                        if obj.icon == "●" || obj.icon == "○" {
                            let c = egui::pos2(text_x + 4.0, mid_y);
                            if obj.icon == "●" { painter.circle_filled(c, 2.6, xsi::TYPE_LABEL()); }
                            else { painter.circle_stroke(c, 2.4, egui::Stroke::new(1.0, xsi::TYPE_LABEL())); }
                            painter.text(egui::pos2(text_x + 13.0, mid_y), egui::Align2::LEFT_CENTER,
                                &obj.name, egui::FontId::proportional(12.0), text_col);
                        } else {
                            painter.text(
                                egui::pos2(text_x, mid_y),
                                egui::Align2::LEFT_CENTER,
                                format!("{} {}", obj.icon, obj.name),
                                egui::FontId::proportional(12.0),
                                text_col,
                            );
                        }

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

/// A small triangle: pointing down when open, right when closed.
fn chevron(painter: &egui::Painter, c: egui::Pos2, open: bool, col: egui::Color32) {
    let r = 3.5;
    let pts = if open {
        vec![egui::pos2(c.x - r, c.y - r * 0.6), egui::pos2(c.x + r, c.y - r * 0.6), egui::pos2(c.x, c.y + r * 0.8)]
    } else {
        vec![egui::pos2(c.x - r * 0.6, c.y - r), egui::pos2(c.x + r * 0.8, c.y), egui::pos2(c.x - r * 0.6, c.y + r)]
    };
    painter.add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
}

/// How far each stack row is indented. A chain of single inputs stays in one
/// column however long it is; only a node with several inputs (a Merge)
/// pushes its branches one step in. Returns the step and whether the row is
/// such a branch point.
pub fn stack_levels(depths: &[usize]) -> Vec<(usize, bool)> {
    let n = depths.len();
    let mut parent = vec![usize::MAX; n];
    let mut kids   = vec![0usize; n];
    let mut path: Vec<usize> = vec![];
    for i in 0..n {
        while path.last().map(|p| depths[*p] >= depths[i]).unwrap_or(false) { path.pop(); }
        if let Some(p) = path.last() { parent[i] = *p; kids[*p] += 1; }
        path.push(i);
    }
    let mut level = vec![0usize; n];
    for i in 0..n {
        if parent[i] != usize::MAX { level[i] = level[parent[i]] + (kids[parent[i]] > 1) as usize; }
    }
    (0..n).map(|i| (level[i], kids[i] > 1)).collect()
}

const STACK_ROW: f32 = 22.0;
const STACK_STEP: f32 = 12.0;

pub fn draw_operator_stack(
    ui:    &mut egui::Ui,
    stack: &mut OperatorStack,
    graph: &mut NodeGraphState,
) {
    egui::Frame::none()
        .fill(xsi::PANEL_BG())
        .inner_margin(6.0)
        .show(ui, |ui| {
            ui.colored_label(xsi::HEADER(),
                egui::RichText::new("Operator Stack").strong().size(14.0));
            ui.separator();

            let depths: Vec<usize> = stack.entries.iter().map(|e| e.depth).collect();
            let levels = stack_levels(&depths);
            let bypassed: std::collections::HashSet<NodeId> =
                graph.nodes.iter().filter(|n| n.bypassed).map(|n| n.id).collect();

            egui::ScrollArea::vertical()
                .id_source("op_stack_scroll")
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let mut toggle_id = None;
                    let mut select_id = None;

                    for (entry, (level, branch)) in stack.entries.iter().zip(levels) {
                        let is_sel = stack.selected_entry == Some(entry.node_id)
                            || (stack.selected_entry.is_none()
                                && graph.selected_node == Some(entry.node_id));
                        let off = bypassed.contains(&entry.node_id);

                        let (row, resp) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), STACK_ROW), egui::Sense::click());
                        let x0 = row.min.x + level.min(10) as f32 * STACK_STEP;
                        let gutter = egui::Rect::from_min_size(egui::pos2(x0, row.min.y), egui::vec2(16.0, STACK_ROW));
                        let over_gutter = resp.hover_pos().map(|p| gutter.contains(p)).unwrap_or(false);
                        if resp.clicked() {
                            if over_gutter && entry.has_children { toggle_id = Some(entry.node_id); }
                            else { select_id = Some(entry.node_id); }
                        }

                        let painter = ui.painter();
                        if is_sel { painter.rect_filled(row, 3.0, xsi::SEL_BG()); }
                        else if resp.hovered() { painter.rect_filled(row, 3.0, xsi::HOVER_BG()); }

                        // The chevron shows at branch points, on collapsed
                        // rows, and under the cursor. Elsewhere: a rail dot.
                        let g = gutter.center();
                        if entry.has_children && (branch || !entry.expanded || resp.hovered()) {
                            chevron(painter, g, entry.expanded, if over_gutter { xsi::NAME() } else { xsi::TYPE_LABEL() });
                        } else {
                            painter.circle_filled(g, 1.6, xsi::DIVIDER());
                        }

                        let name_col = if off { xsi::TYPE_LABEL() } else if is_sel { xsi::SEL_NAME() } else { xsi::NAME() };
                        let name = painter.text(
                            egui::pos2(x0 + 18.0, row.center().y), egui::Align2::LEFT_CENTER,
                            format!("{} {}", entry.type_icon, entry.name),
                            egui::FontId::proportional(12.5), name_col);
                        if off {
                            painter.line_segment([name.left_center(), name.right_center()], egui::Stroke::new(1.0, xsi::TYPE_LABEL()));
                        }
                        // Type at the right, when there is room for it.
                        let label = if off { "bypassed" } else { entry.type_label };
                        let galley = painter.layout_no_wrap(label.to_string(), egui::FontId::proportional(9.5), xsi::TYPE_LABEL());
                        if name.right() + 10.0 + galley.size().x < row.max.x - 6.0 && entry.type_label != entry.name {
                            painter.galley(egui::pos2(row.max.x - 6.0 - galley.size().x, row.center().y - galley.size().y * 0.5),
                                galley, xsi::TYPE_LABEL());
                        }
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

#[cfg(test)]
mod tests {
    use super::stack_levels;

    #[test]
    fn a_long_chain_stays_in_one_column() {
        let depths: Vec<usize> = (0..300).collect();
        assert!(stack_levels(&depths).iter().all(|(level, branch)| *level == 0 && !*branch));
    }

    #[test]
    fn only_branches_step_in() {
        // Output > Merge > (A > A2, B)
        let l = stack_levels(&[0, 1, 2, 3, 2]);
        assert_eq!(l.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 0, 1, 1, 1]);
        assert_eq!(l.iter().map(|x| x.1).collect::<Vec<_>>(), vec![false, true, false, false, false]);
    }
}
