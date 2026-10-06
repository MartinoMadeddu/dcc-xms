use bevy_egui::egui;
use crate::types::{MeshData, PrimInspectorState, PrimInspectorTab, PrimVarInterp};
use crate::node_graph::NodeGraphState;

const ROW_H:      f32 = 20.0;
const IDX_COL_W:  f32 = 40.0;
const DATA_COL_W: f32 = 90.0;
const HEADER_H:   f32 = 24.0;
const MAX_ROWS:   usize = 200; // max rows rendered at once (virtual scroll)

mod xsi {
    // Light-theme colours. `theme::c` returns the dark counterpart in dark mode.
    #![allow(non_snake_case)]
    use bevy_egui::egui::Color32;
    pub fn BG() -> Color32 { crate::theme::c(72, 72, 72) }
    pub fn HEADER_BG() -> Color32 { crate::theme::c(58, 58, 58) }
    pub fn ROW_EVEN() -> Color32 { crate::theme::c(72, 72, 72) }
    pub fn ROW_ODD() -> Color32 { crate::theme::c(66, 66, 66) }
    pub fn BORDER() -> Color32 { crate::theme::c(50, 50, 50) }
    pub fn TEXT() -> Color32 { crate::theme::c(210, 210, 210) }
    pub fn TEXT_DIM() -> Color32 { crate::theme::c(150, 150, 150) }
    pub fn TEXT_IDX() -> Color32 { crate::theme::c(120, 130, 145) }
    pub fn TAB_ACTIVE() -> Color32 { crate::theme::c(80, 95, 115) }
    pub fn TAB_BG() -> Color32 { crate::theme::c(58, 58, 58) }
    pub fn BREADCRUMB() -> Color32 { crate::theme::c(160, 165, 170) }
    pub fn PATH_BG() -> Color32 { crate::theme::c(45, 45, 45) }
}

pub fn draw_prim_inspector(
    ui:       &mut egui::Ui,
    graph:    &NodeGraphState,
    state:    &mut PrimInspectorState,
    // Graph revision: the table is rebuilt only when this or the selected
    // node changes, not on every frame.
    revision: u64,
    get_mesh: &dyn Fn(&NodeGraphState) -> Option<MeshData>,
) {
    egui::Frame::none()
        .fill(xsi::BG())
        .show(ui, |ui| {
            let selected_node = graph.selected_node;
            if !state.is_cache_valid(selected_node, revision) {
                state.update_cache(selected_node, get_mesh(graph), revision);
            }

            // ── Path breadcrumb ───────────────────────────────────────────────
            let path_str = if let Some(id) = selected_node {
                graph.nodes.iter()
                    .find(|n| n.id == id)
                    .map(|n| format!("/{}", n.name))
                    .unwrap_or_else(|| "/".into())
            } else {
                String::new()
            };

            egui::Frame::none()
                .fill(xsi::PATH_BG())
                .inner_margin(egui::vec2(8.0, 4.0))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(&path_str)
                            .color(xsi::BREADCRUMB())
                            .monospace()
                            .size(10.0),
                    );
                });

            let Some(mesh) = state.cached_mesh() else {
                ui.add_space(12.0);
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new("No mesh data for selected node.")
                        .color(xsi::TEXT_DIM()));
                });
                return;
            };

            // ── Tabs ─────────────────────────────────────────────────────────
            let count = |interp: PrimVarInterp| mesh.primvars.iter().filter(|p| p.interp == interp).count();
            let vertex_count = mesh.vertices.len();
            let face_count   = mesh.num_faces();
            let fv_count     = mesh.num_face_varying();
            let const_count  = count(PrimVarInterp::Constant);

            egui::Frame::none()
                .fill(xsi::TAB_BG())
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        tab_btn(ui, state, PrimInspectorTab::Vertex,
                            &format!("Vertex ({})", vertex_count));
                        tab_btn(ui, state, PrimInspectorTab::Uniform,
                            &format!("Uniform ({})", face_count));
                        tab_btn(ui, state, PrimInspectorTab::FaceVarying,
                            &format!("FaceVarying ({})", fv_count));
                        tab_btn(ui, state, PrimInspectorTab::Constant,
                            &format!("Constant ({})", const_count));
                    });
                });

            ui.separator();

            // ── Spreadsheet ───────────────────────────────────────────────────
            // The columns of the open tab are built once and kept until the
            // mesh or the tab changes. Each frame only draws the visible rows.
            let tab = state.active_tab.clone();
            if !state.table_is_for(&tab) {
                let (rows, cols) = build_table(&mesh, &tab);
                state.set_table(tab.clone(), rows, cols);
            }
            let (rows, cols) = state.take_table();
            if cols.is_empty() {
                ui.add_space(12.0);
                let what = match tab {
                    PrimInspectorTab::FaceVarying => "No FaceVarying primvars.",
                    PrimInspectorTab::Constant    => "No Constant primvars.",
                    _                             => "No data.",
                };
                ui.label(egui::RichText::new(what).color(xsi::TEXT_DIM()));
            } else {
                draw_spreadsheet(ui, state, rows, &cols);
            }
            state.put_table(rows, cols);
        });
}

/// Row count and columns of one tab.
fn build_table(mesh: &MeshData, tab: &PrimInspectorTab) -> (usize, Vec<(String, Vec<Vec<f32>>)>) {
    let vars = |interp: PrimVarInterp| mesh.primvars.iter().filter(move |p| p.interp == interp);
    match tab {
        PrimInspectorTab::Vertex => {
            let mut cols: Vec<(String, Vec<Vec<f32>>)> = vec![
                ("P".into(), mesh.vertices.iter().map(|v| vec![v[0], v[1], v[2]]).collect()),
            ];
            if !mesh.normals.is_empty() {
                cols.push(("N".into(), mesh.normals.iter().map(|n| vec![n[0], n[1], n[2]]).collect()));
            }
            for pv in vars(PrimVarInterp::Vertex) {
                if pv.name != "N" { cols.push((pv.name.clone(), pv.values.clone())); }
            }
            (mesh.vertices.len(), cols)
        }
        PrimInspectorTab::Uniform => {
            let face_count = mesh.num_faces();
            let corner = |k: usize| -> Vec<Vec<f32>> {
                (0..face_count).map(|f| vec![mesh.indices.get(f * 3 + k).copied().unwrap_or(0) as f32]).collect()
            };
            let mut cols = vec![("vtx[0]".to_string(), corner(0)), ("vtx[1]".to_string(), corner(1)), ("vtx[2]".to_string(), corner(2))];
            for pv in vars(PrimVarInterp::Uniform) { cols.push((pv.name.clone(), pv.values.clone())); }
            (face_count, cols)
        }
        PrimInspectorTab::FaceVarying => (
            mesh.num_face_varying(),
            vars(PrimVarInterp::FaceVarying).map(|pv| (pv.name.clone(), pv.values.clone())).collect(),
        ),
        PrimInspectorTab::Constant => {
            let cols: Vec<_> = vars(PrimVarInterp::Constant).map(|pv| (pv.name.clone(), pv.values.clone())).collect();
            (cols.len(), cols)
        }
    }
}

// ── Tab button ────────────────────────────────────────────────────────────────
// Update tab_btn to use the new method
fn tab_btn(ui: &mut egui::Ui, state: &mut PrimInspectorState, tab: PrimInspectorTab, label: &str) {
    let is_active = state.active_tab == tab;
    let btn = egui::Button::new(
        egui::RichText::new(label)
            .color(if is_active { egui::Color32::WHITE } else { xsi::TEXT_DIM() })
            .size(11.0),
    )
    .fill(if is_active { xsi::TAB_ACTIVE() } else { egui::Color32::TRANSPARENT })
    .frame(true)
    .min_size(egui::vec2(0.0, 22.0));

    if ui.add(btn).clicked() {
        state.set_active_tab(tab);  // ✅ Use new method
    }
}



// ── Spreadsheet grid ──────────────────────────────────────────────────────────

/// `cols` is a list of (column_group_name, per_row_components).
/// Components wider than 1 are shown as name.X / name.Y / name.Z.
fn draw_spreadsheet(
    ui:        &mut egui::Ui,
    state:     &mut PrimInspectorState,
    row_count: usize,
    cols:      &[(String, Vec<Vec<f32>>)],
) {
    if row_count == 0 || cols.is_empty() {
        ui.label(egui::RichText::new("No data.").color(xsi::TEXT_DIM()));
        return;
    }

    // Expand column headers: P with width 3 → P.X, P.Y, P.Z
    struct ColDef<'a> { header: String, source: &'a str, component: usize }
    let mut col_defs: Vec<ColDef> = Vec::new();
    for (name, rows) in cols {
        let width = rows.first().map(|r| r.len()).unwrap_or(1);
        if width == 1 {
            col_defs.push(ColDef { header: name.to_string(), source: name, component: 0 });
        } else {
            let suffixes = ["X","Y","Z","W"];
            for c in 0..width {
                col_defs.push(ColDef {
                    header:    format!("{}.{}", name, suffixes.get(c).unwrap_or(&"?")),
                    source:    name,
                    component: c,
                });
            }
        }
    }

    let total_w = IDX_COL_W + col_defs.len() as f32 * DATA_COL_W;
    let avail_h = ui.available_height();

    // Virtual scroll: how many rows fit?
    let visible_rows = ((avail_h - HEADER_H) / ROW_H).floor() as usize;
    let visible_rows = visible_rows.min(MAX_ROWS).min(row_count);

    // Clamp row_offset
    if state.row_offset + visible_rows > row_count {
        state.row_offset = row_count.saturating_sub(visible_rows);
    }

    let scroll_area = egui::ScrollArea::horizontal()
        .id_source("prim_inspector_hscroll")
        .auto_shrink([false, false]);

    scroll_area.show(ui, |ui| {
        let (outer_rect, _) = ui.allocate_exact_size(
            egui::vec2(total_w.max(ui.available_width()), avail_h),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(outer_rect);

        // ── Header row ────────────────────────────────────────────────────────
        let hdr_rect = egui::Rect::from_min_size(
            outer_rect.min,
            egui::vec2(outer_rect.width(), HEADER_H),
        );
        painter.rect_filled(hdr_rect, 0.0, xsi::HEADER_BG());

        // Index column header
        painter.rect_stroke(
            egui::Rect::from_min_size(hdr_rect.min, egui::vec2(IDX_COL_W, HEADER_H)),
            0.0, egui::Stroke::new(1.0, xsi::BORDER()),
        );

        for (ci, cd) in col_defs.iter().enumerate() {
            let x = outer_rect.min.x + IDX_COL_W + ci as f32 * DATA_COL_W;
            let cell = egui::Rect::from_min_size(
                egui::pos2(x, outer_rect.min.y),
                egui::vec2(DATA_COL_W, HEADER_H),
            );
            painter.rect_stroke(cell, 0.0, egui::Stroke::new(1.0, xsi::BORDER()));
            painter.text(
                cell.center(),
                egui::Align2::CENTER_CENTER,
                &cd.header,
                egui::FontId::proportional(11.0),
                xsi::TEXT(),
            );
        }

        // ── Data rows ─────────────────────────────────────────────────────────
        for (ri, row_idx) in (state.row_offset..state.row_offset + visible_rows).enumerate() {
            let y = outer_rect.min.y + HEADER_H + ri as f32 * ROW_H;
            let row_col = if ri % 2 == 0 { xsi::ROW_EVEN() } else { xsi::ROW_ODD() };

            let row_rect = egui::Rect::from_min_size(
                egui::pos2(outer_rect.min.x, y),
                egui::vec2(outer_rect.width(), ROW_H),
            );
            painter.rect_filled(row_rect, 0.0, row_col);

            // Index cell
            let idx_cell = egui::Rect::from_min_size(
                egui::pos2(outer_rect.min.x, y),
                egui::vec2(IDX_COL_W, ROW_H),
            );
            painter.rect_stroke(idx_cell, 0.0, egui::Stroke::new(1.0, xsi::BORDER()));
            painter.text(
                idx_cell.center(),
                egui::Align2::CENTER_CENTER,
                &row_idx.to_string(),
                egui::FontId::proportional(10.0),
                xsi::TEXT_IDX(),
            );

            // Data cells
            for (ci, cd) in col_defs.iter().enumerate() {
                let x = outer_rect.min.x + IDX_COL_W + ci as f32 * DATA_COL_W;
                let cell = egui::Rect::from_min_size(
                    egui::pos2(x, y),
                    egui::vec2(DATA_COL_W, ROW_H),
                );
                painter.rect_stroke(cell, 0.0, egui::Stroke::new(1.0, xsi::BORDER()));

                // Find value
                let val_str = cols.iter()
                    .find(|(n, _)| *n == cd.source)
                    .and_then(|(_, rows)| rows.get(row_idx))
                    .and_then(|r| r.get(cd.component))
                    .map(|v| format!("{:.4}", v))
                    .unwrap_or_else(|| "-".into());

                painter.text(
                    egui::pos2(cell.max.x - 6.0, cell.center().y),
                    egui::Align2::RIGHT_CENTER,
                    &val_str,
                    egui::FontId::monospace(10.0),
                    xsi::TEXT(),
                );
            }
        }

        // ── Vertical scroll bar (manual) ──────────────────────────────────────
        if row_count > visible_rows {
            let sb_w    = 8.0;
            let sb_rect = egui::Rect::from_min_size(
                egui::pos2(outer_rect.max.x - sb_w, outer_rect.min.y + HEADER_H),
                egui::vec2(sb_w, avail_h - HEADER_H),
            );
            painter.rect_filled(sb_rect, 4.0, egui::Color32::from_gray(50));

            let ratio    = visible_rows as f32 / row_count as f32;
            let thumb_h  = (sb_rect.height() * ratio).max(20.0);
            let thumb_y  = sb_rect.min.y
                + (sb_rect.height() - thumb_h)
                * (state.row_offset as f32 / (row_count - visible_rows) as f32);
            let thumb = egui::Rect::from_min_size(
                egui::pos2(sb_rect.min.x, thumb_y),
                egui::vec2(sb_w, thumb_h),
            );
            painter.rect_filled(thumb, 4.0, egui::Color32::from_gray(110));

            // Scroll on mouse wheel inside the panel
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_delta != 0.0 && outer_rect.contains(
                ui.input(|i| i.pointer.hover_pos()).unwrap_or_default()
            ) {
                let lines = (-scroll_delta / ROW_H).round() as i64;
                state.row_offset = (state.row_offset as i64 + lines)
                    .clamp(0, (row_count - visible_rows) as i64) as usize;
            }
        }
    });
}