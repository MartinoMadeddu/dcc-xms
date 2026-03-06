use bevy_egui::egui;
use crate::types::{MeshData, PrimInspectorState, PrimInspectorTab, PrimVarInterp};
use crate::node_graph::NodeGraphState;

const ROW_H:      f32 = 20.0;
const IDX_COL_W:  f32 = 40.0;
const DATA_COL_W: f32 = 90.0;
const HEADER_H:   f32 = 24.0;
const MAX_ROWS:   usize = 200; // max rows rendered at once (virtual scroll)

mod xsi {
    use bevy_egui::egui::Color32;
    pub const BG:         Color32 = Color32::from_rgb(72, 72, 72);
    pub const HEADER_BG:  Color32 = Color32::from_rgb(58, 58, 58);
    pub const ROW_EVEN:   Color32 = Color32::from_rgb(72, 72, 72);
    pub const ROW_ODD:    Color32 = Color32::from_rgb(66, 66, 66);
    pub const BORDER:     Color32 = Color32::from_rgb(50, 50, 50);
    pub const TEXT:       Color32 = Color32::from_rgb(210, 210, 210);
    pub const TEXT_DIM:   Color32 = Color32::from_rgb(150, 150, 150);
    pub const TEXT_IDX:   Color32 = Color32::from_rgb(120, 130, 145);
    pub const TAB_ACTIVE: Color32 = Color32::from_rgb(80, 95, 115);
    pub const TAB_BG:     Color32 = Color32::from_rgb(58, 58, 58);
    pub const BREADCRUMB: Color32 = Color32::from_rgb(160, 165, 170);
    pub const PATH_BG:    Color32 = Color32::from_rgb(45, 45, 45);
}

pub fn draw_prim_inspector(
    ui:      &mut egui::Ui,
    graph:   &NodeGraphState,
    state:   &mut PrimInspectorState,
    get_mesh: &dyn Fn(&NodeGraphState) -> Option<MeshData>,
) {
    egui::Frame::none()
        .fill(xsi::BG)
        .show(ui, |ui| {
            let mesh_opt = get_mesh(graph);

            // ── Path breadcrumb ───────────────────────────────────────────────
            let path_str = if let Some(id) = graph.selected_node {
                graph.nodes.iter()
                    .find(|n| n.id == id)
                    .map(|n| format!("/{}", n.name))
                    .unwrap_or_else(|| "/".into())
            } else {
                String::new()
            };

            egui::Frame::none()
                .fill(xsi::PATH_BG)
                .inner_margin(egui::vec2(8.0, 4.0))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(&path_str)
                            .color(xsi::BREADCRUMB)
                            .monospace()
                            .size(10.0),
                    );
                });

            let Some(mesh) = mesh_opt else {
                ui.add_space(12.0);
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new("No mesh data for selected node.")
                        .color(xsi::TEXT_DIM));
                });
                return;
            };

            // ── Tabs ─────────────────────────────────────────────────────────
            let vertex_vars: Vec<_> = mesh.primvars.iter()
                .filter(|p| p.interp == PrimVarInterp::Vertex).collect();
            let uniform_vars: Vec<_> = mesh.primvars.iter()
                .filter(|p| p.interp == PrimVarInterp::Uniform).collect();
            let fv_vars: Vec<_> = mesh.primvars.iter()
                .filter(|p| p.interp == PrimVarInterp::FaceVarying).collect();
            let const_vars: Vec<_> = mesh.primvars.iter()
                .filter(|p| p.interp == PrimVarInterp::Constant).collect();

            let vertex_count = mesh.vertices.len();
            let face_count   = mesh.num_faces();
            let fv_count     = mesh.num_face_varying();

            egui::Frame::none()
                .fill(xsi::TAB_BG)
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
                            &format!("Constant ({})", const_vars.len()));
                    });
                });

            ui.separator();

            // ── Spreadsheet ───────────────────────────────────────────────────
            match state.active_tab {
                PrimInspectorTab::Vertex => {
                    // Always show P (position), then N if present, then other vertex vars
                    let mut cols: Vec<(&str, Vec<Vec<f32>>)> = vec![
                        ("P", mesh.vertices.iter()
                            .map(|v| vec![v[0], v[1], v[2]]).collect()),
                    ];
                    for pv in &vertex_vars {
                        if pv.name != "N" {
                            cols.push((&pv.name, pv.values.clone()));
                        }
                    }
                    // Insert N after P if it exists
                    if !mesh.normals.is_empty() {
                        cols.insert(1, ("N", mesh.normals.iter()
                            .map(|n| vec![n[0], n[1], n[2]]).collect()));
                    }
                    draw_spreadsheet(ui, state, vertex_count, &cols);
                }
                PrimInspectorTab::Uniform => {
                    // Show face indices as rows, face primvars as columns
                    // Reconstruct faces from triangulated indices (groups of 3)
                    let face_rows: Vec<Vec<f32>> = (0..face_count)
                        .map(|f| {
                            let base = f * 3;
                            if base + 2 < mesh.indices.len() {
                                vec![
                                    mesh.indices[base]   as f32,
                                    mesh.indices[base+1] as f32,
                                    mesh.indices[base+2] as f32,
                                ]
                            } else { vec![] }
                        })
                        .collect();
                    let mut cols: Vec<(&str, Vec<Vec<f32>>)> = vec![
                        ("vtx[0]", face_rows.iter().map(|r| vec![*r.get(0).unwrap_or(&0.0)]).collect()),
                        ("vtx[1]", face_rows.iter().map(|r| vec![*r.get(1).unwrap_or(&0.0)]).collect()),
                        ("vtx[2]", face_rows.iter().map(|r| vec![*r.get(2).unwrap_or(&0.0)]).collect()),
                    ];
                    for pv in &uniform_vars {
                        cols.push((&pv.name, pv.values.clone()));
                    }
                    draw_spreadsheet(ui, state, face_count, &cols);
                }
                PrimInspectorTab::FaceVarying => {
                    let mut cols: Vec<(&str, Vec<Vec<f32>>)> = Vec::new();
                    for pv in &fv_vars {
                        cols.push((&pv.name, pv.values.clone()));
                    }
                    if cols.is_empty() {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new("No FaceVarying primvars.")
                            .color(xsi::TEXT_DIM));
                    } else {
                        draw_spreadsheet(ui, state, fv_count, &cols);
                    }
                }
                PrimInspectorTab::Constant => {
                    let mut cols: Vec<(&str, Vec<Vec<f32>>)> = Vec::new();
                    for pv in &const_vars {
                        cols.push((&pv.name, pv.values.clone()));
                    }
                    if cols.is_empty() {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new("No Constant primvars.")
                            .color(xsi::TEXT_DIM));
                    } else {
                        draw_spreadsheet(ui, state, const_vars.len(), &cols);
                    }
                }
            }
        });
}

// ── Tab button ────────────────────────────────────────────────────────────────

fn tab_btn(ui: &mut egui::Ui, state: &mut PrimInspectorState, tab: PrimInspectorTab, label: &str) {
    let is_active = state.active_tab == tab;
    let btn = egui::Button::new(
        egui::RichText::new(label)
            .color(if is_active { egui::Color32::WHITE } else { xsi::TEXT_DIM })
            .size(11.0),
    )
    .fill(if is_active { xsi::TAB_ACTIVE } else { egui::Color32::TRANSPARENT })
    .frame(true)
    .min_size(egui::vec2(0.0, 22.0));

    if ui.add(btn).clicked() {
        state.active_tab  = tab;
        state.row_offset  = 0;
    }
}

// ── Spreadsheet grid ──────────────────────────────────────────────────────────

/// `cols` is a list of (column_group_name, per_row_components).
/// Components wider than 1 are shown as name.X / name.Y / name.Z.
fn draw_spreadsheet(
    ui:        &mut egui::Ui,
    state:     &mut PrimInspectorState,
    row_count: usize,
    cols:      &[(&str, Vec<Vec<f32>>)],
) {
    if row_count == 0 || cols.is_empty() {
        ui.label(egui::RichText::new("No data.").color(xsi::TEXT_DIM));
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
        painter.rect_filled(hdr_rect, 0.0, xsi::HEADER_BG);

        // Index column header
        painter.rect_stroke(
            egui::Rect::from_min_size(hdr_rect.min, egui::vec2(IDX_COL_W, HEADER_H)),
            0.0, egui::Stroke::new(1.0, xsi::BORDER),
        );

        for (ci, cd) in col_defs.iter().enumerate() {
            let x = outer_rect.min.x + IDX_COL_W + ci as f32 * DATA_COL_W;
            let cell = egui::Rect::from_min_size(
                egui::pos2(x, outer_rect.min.y),
                egui::vec2(DATA_COL_W, HEADER_H),
            );
            painter.rect_stroke(cell, 0.0, egui::Stroke::new(1.0, xsi::BORDER));
            painter.text(
                cell.center(),
                egui::Align2::CENTER_CENTER,
                &cd.header,
                egui::FontId::proportional(11.0),
                xsi::TEXT,
            );
        }

        // ── Data rows ─────────────────────────────────────────────────────────
        for (ri, row_idx) in (state.row_offset..state.row_offset + visible_rows).enumerate() {
            let y = outer_rect.min.y + HEADER_H + ri as f32 * ROW_H;
            let row_col = if ri % 2 == 0 { xsi::ROW_EVEN } else { xsi::ROW_ODD };

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
            painter.rect_stroke(idx_cell, 0.0, egui::Stroke::new(1.0, xsi::BORDER));
            painter.text(
                idx_cell.center(),
                egui::Align2::CENTER_CENTER,
                &row_idx.to_string(),
                egui::FontId::proportional(10.0),
                xsi::TEXT_IDX,
            );

            // Data cells
            for (ci, cd) in col_defs.iter().enumerate() {
                let x = outer_rect.min.x + IDX_COL_W + ci as f32 * DATA_COL_W;
                let cell = egui::Rect::from_min_size(
                    egui::pos2(x, y),
                    egui::vec2(DATA_COL_W, ROW_H),
                );
                painter.rect_stroke(cell, 0.0, egui::Stroke::new(1.0, xsi::BORDER));

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
                    xsi::TEXT,
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