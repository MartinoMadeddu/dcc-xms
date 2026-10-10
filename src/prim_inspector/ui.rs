use bevy_egui::egui;
use crate::core::anim::AnimData;
use crate::types::{InspectorTable, MeshData, PrimInspectorState, PrimInspectorTab, PrimVarInterp};
use crate::node_graph::NodeGraphState;

const ROW_H:      f32 = 20.0;
const IDX_COL_W:  f32 = 40.0;
const DATA_COL_W: f32 = 90.0;
const LABEL_COL_W: f32 = 230.0;
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
    pub fn TEXT() -> Color32 { crate::theme::c(228, 228, 228) }
    pub fn TEXT_DIM() -> Color32 { crate::theme::c(210, 210, 210) }
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
    // Graph revision: the data is cooked again only when this or the
    // selected node changes, not on every frame.
    revision: u64,
    // Playhead, for the pose a clip is listed in.
    time:     f64,
    get_mesh: &dyn Fn(&NodeGraphState) -> Option<MeshData>,
    get_clip: &dyn Fn(&NodeGraphState) -> Option<std::sync::Arc<AnimData>>,
) {
    egui::Frame::none()
        .fill(xsi::BG())
        .show(ui, |ui| {
            let selected_node = graph.selected_node;
            if !state.is_cache_valid(selected_node, revision) {
                let clip = get_clip(graph);
                let mesh = if clip.is_none() { get_mesh(graph) } else { None };
                state.update_cache(selected_node, mesh, clip, revision);
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

            let clip = state.cached_clip();
            let mesh = state.cached_mesh().filter(|m| !m.vertices.is_empty() || !m.points.is_empty());
            if clip.is_none() && mesh.is_none() {
                ui.add_space(12.0);
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new("No mesh or clip on the selected node.")
                        .color(xsi::TEXT_DIM()));
                });
                return;
            }

            // ── Tabs ─────────────────────────────────────────────────────────
            // A clip lists joints and bones; a mesh lists its components.
            use PrimInspectorTab as Tab;
            let clip_tab = matches!(state.active_tab, Tab::Joint | Tab::Bone);
            if clip.is_some() && !clip_tab { state.set_active_tab(Tab::Joint); }
            if clip.is_none() && clip_tab { state.set_active_tab(Tab::Vertex); }

            egui::Frame::none()
                .fill(xsi::TAB_BG())
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if let Some(c) = &clip {
                            tab_btn(ui, state, Tab::Joint, &format!("Joint ({})", c.joints.len()));
                            tab_btn(ui, state, Tab::Bone, &format!("Bone ({})", bones(c).len()));
                        } else if let Some(m) = &mesh {
                            let count = |interp: PrimVarInterp| m.primvars.iter().filter(|p| p.interp == interp).count();
                            let verts = if m.vertices.is_empty() { m.points.len() } else { m.vertices.len() };
                            tab_btn(ui, state, Tab::Vertex, &format!("Vertex ({verts})"));
                            tab_btn(ui, state, Tab::Edge, &format!("Edge ({})", state.edge_count));
                            tab_btn(ui, state, Tab::Uniform, &format!("Polygon ({})", m.num_faces()));
                            tab_btn(ui, state, Tab::FaceVarying, &format!("FaceVarying ({})", m.num_face_varying()));
                            tab_btn(ui, state, Tab::Constant, &format!("Constant ({})", count(PrimVarInterp::Constant)));
                        }
                    });
                });

            ui.separator();

            // ── Spreadsheet ───────────────────────────────────────────────────
            // The open tab is built once and kept until the data, the tab or
            // (for a clip) the frame changes. Each frame draws the visible rows.
            let tab = state.active_tab.clone();
            let frame = clip.as_ref().map(|c| c.index_at(time)).unwrap_or(0);
            if !state.table_is_for(&tab, frame) {
                let table = match (&clip, &mesh) {
                    (Some(c), _) => clip_table(c, &tab, frame),
                    (None, Some(m)) => mesh_table(m, &tab),
                    _ => InspectorTable::default(),
                };
                state.set_table(tab.clone(), frame, table);
            }
            let table = state.take_table();
            if table.cols.is_empty() {
                ui.add_space(12.0);
                let what = match tab {
                    Tab::FaceVarying => "No FaceVarying data. A UV Unwrap node adds UVs.",
                    Tab::Constant    => "No Constant primvars.",
                    _                => "No data.",
                };
                ui.label(egui::RichText::new(what).color(xsi::TEXT_DIM()));
            } else {
                draw_spreadsheet(ui, state, &table);
            }
            state.put_table(table);
        });
}

/// Bones of a skeleton: one from each joint to each bone joint below it.
pub fn bones(clip: &AnimData) -> Vec<(usize, usize)> {
    clip.joints.iter().enumerate()
        .filter(|(_, j)| j.is_bone)
        .filter_map(|(c, j)| j.parent.map(|p| (p, c)))
        .collect()
}

/// Joints or bones of a clip in the pose of one frame.
pub fn clip_table(clip: &AnimData, tab: &PrimInspectorTab, frame: usize) -> InspectorTable {
    let world = clip.world_pose(frame);
    let pos = |j: usize| world[j].w_axis.truncate();
    match tab {
        PrimInspectorTab::Bone => {
            let list = bones(clip);
            let labels = list.iter().map(|(a, b)| format!("{} > {}", clip.joints[*a].name, clip.joints[*b].name)).collect();
            InspectorTable {
                rows: list.len(),
                labels: Some(("from > to".into(), labels)),
                cols: vec![
                    ("from".into(), list.iter().map(|(a, _)| vec![*a as f32]).collect()),
                    ("to".into(), list.iter().map(|(_, b)| vec![*b as f32]).collect()),
                    ("length".into(), list.iter().map(|(a, b)| vec![pos(*a).distance(pos(*b))]).collect()),
                    ("dir".into(), list.iter().map(|(a, b)| (pos(*b) - pos(*a)).normalize_or_zero().to_array().to_vec()).collect()),
                ],
            }
        }
        _ => {
            let n = clip.joints.len();
            let euler = |j: usize| {
                let (x, y, z) = clip.local(j, frame).rotation.to_euler(bevy::math::EulerRot::XYZ);
                vec![x.to_degrees(), y.to_degrees(), z.to_degrees()]
            };
            InspectorTable {
                rows: n,
                labels: Some(("joint".into(), clip.joints.iter().map(|j| j.name.clone()).collect())),
                cols: vec![
                    ("parent".into(), clip.joints.iter().map(|j| vec![j.parent.map(|p| p as f32).unwrap_or(-1.0)]).collect()),
                    ("P".into(), (0..n).map(|j| pos(j).to_array().to_vec()).collect()),
                    ("R".into(), (0..n).map(euler).collect()),
                    ("bone".into(), clip.joints.iter().map(|j| vec![j.is_bone as u8 as f32]).collect()),
                ],
            }
        }
    }
}

/// One component tab of a mesh.
pub fn mesh_table(mesh: &MeshData, tab: &PrimInspectorTab) -> InspectorTable {
    use bevy::math::Vec3;
    let vars = |interp: PrimVarInterp| mesh.primvars.iter().filter(move |p| p.interp == interp);
    let p = |v: u32| Vec3::from_array(mesh.vertices[v as usize]);
    let (rows, cols): (usize, Vec<(String, Vec<Vec<f32>>)>) = match tab {
        PrimInspectorTab::Vertex if mesh.vertices.is_empty() => (
            // A point cloud.
            mesh.points.len(),
            vec![("P".into(), mesh.points.iter().map(|v| v.to_vec()).collect())],
        ),
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
        PrimInspectorTab::Edge => {
            let edges = crate::core::poly::PolyMesh::from_mesh(mesh).edges();
            (edges.len(), vec![
                ("vtx[0]".into(), edges.iter().map(|e| vec![e[0] as f32]).collect()),
                ("vtx[1]".into(), edges.iter().map(|e| vec![e[1] as f32]).collect()),
                ("length".into(), edges.iter().map(|e| vec![p(e[0]).distance(p(e[1]))]).collect()),
            ])
        }
        PrimInspectorTab::Uniform => {
            let poly = crate::core::poly::PolyMesh::from_mesh(mesh);
            let n = poly.polys.len();
            let corner = |k: usize| -> Vec<Vec<f32>> {
                poly.polys.iter().map(|q| vec![q.get(k).map(|v| *v as f32).unwrap_or(-1.0)]).collect()
            };
            let mut cols: Vec<(String, Vec<Vec<f32>>)> = vec![
                ("corners".into(), poly.polys.iter().map(|q| vec![q.len() as f32]).collect()),
                ("vtx[0]".into(), corner(0)), ("vtx[1]".into(), corner(1)),
                ("vtx[2]".into(), corner(2)), ("vtx[3]".into(), corner(3)),
                ("N".into(), (0..n).map(|q| poly.normal(q).to_array().to_vec()).collect()),
                ("area".into(), (0..n).map(|q| vec![poly.area_normal(q).length() * 0.5]).collect()),
            ];
            for pv in vars(PrimVarInterp::Uniform) { cols.push((pv.name.clone(), pv.values.clone())); }
            (n, cols)
        }
        PrimInspectorTab::FaceVarying => {
            let mut cols: Vec<(String, Vec<Vec<f32>>)> = vec![];
            if mesh.uvs.len() == mesh.indices.len() && !mesh.uvs.is_empty() {
                cols.push(("vtx".into(), mesh.indices.iter().map(|i| vec![*i as f32]).collect()));
                cols.push(("uv".into(), mesh.uvs.iter().map(|uv| uv.to_vec()).collect()));
            }
            cols.extend(vars(PrimVarInterp::FaceVarying).map(|pv| (pv.name.clone(), pv.values.clone())));
            (mesh.num_face_varying(), cols)
        }
        PrimInspectorTab::Constant => {
            let cols: Vec<_> = vars(PrimVarInterp::Constant).map(|pv| (pv.name.clone(), pv.values.clone())).collect();
            (cols.len(), cols)
        }
        _ => (0, vec![]),
    };
    InspectorTable { rows, labels: None, cols }
}

// ── Tab button ────────────────────────────────────────────────────────────────
// Update tab_btn to use the new method
fn tab_btn(ui: &mut egui::Ui, state: &mut PrimInspectorState, tab: PrimInspectorTab, label: &str) {
    let is_active = state.active_tab == tab;
    let btn = egui::Button::new(
        egui::RichText::new(label)
            .color(if is_active { xsi::TEXT() } else { xsi::TEXT_DIM() })
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
    ui:    &mut egui::Ui,
    state: &mut PrimInspectorState,
    table: &InspectorTable,
) {
    let row_count = table.rows;
    let cols = &table.cols;
    // Width of the text column, when the table has one.
    let label_w = if table.labels.is_some() { LABEL_COL_W } else { 0.0 };
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

    let total_w = IDX_COL_W + label_w + col_defs.len() as f32 * DATA_COL_W;
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

        if let Some((heading, _)) = &table.labels {
            let cell = egui::Rect::from_min_size(
                egui::pos2(outer_rect.min.x + IDX_COL_W, outer_rect.min.y), egui::vec2(label_w, HEADER_H));
            painter.rect_stroke(cell, 0.0, egui::Stroke::new(1.0, xsi::BORDER()));
            painter.text(cell.center(), egui::Align2::CENTER_CENTER, heading, egui::FontId::proportional(11.0), xsi::TEXT());
        }

        for (ci, cd) in col_defs.iter().enumerate() {
            let x = outer_rect.min.x + IDX_COL_W + label_w + ci as f32 * DATA_COL_W;
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

            // Text cell
            if let Some((_, labels)) = &table.labels {
                let cell = egui::Rect::from_min_size(egui::pos2(outer_rect.min.x + IDX_COL_W, y), egui::vec2(label_w, ROW_H));
                painter.rect_stroke(cell, 0.0, egui::Stroke::new(1.0, xsi::BORDER()));
                painter.with_clip_rect(cell.shrink(2.0)).text(
                    egui::pos2(cell.min.x + 6.0, cell.center().y), egui::Align2::LEFT_CENTER,
                    labels.get(row_idx).map(|s| s.as_str()).unwrap_or("-"),
                    egui::FontId::proportional(10.5), xsi::TEXT());
            }

            // Data cells
            for (ci, cd) in col_defs.iter().enumerate() {
                let x = outer_rect.min.x + IDX_COL_W + label_w + ci as f32 * DATA_COL_W;
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::anim::{create_test_clip, FrameRate};
    use crate::node_graph::nodes::{create_cube, create_grid};

    fn col<'a>(t: &'a InspectorTable, name: &str) -> &'a Vec<Vec<f32>> { &t.cols.iter().find(|c| c.0 == name).unwrap().1 }

    #[test]
    fn a_mesh_lists_vertices_edges_and_polygons() {
        let cube = create_cube(1.0);
        let v = mesh_table(&cube, &PrimInspectorTab::Vertex);
        assert_eq!(v.rows, 8);
        assert_eq!(col(&v, "P")[1], vec![0.5, -0.5, -0.5]);
        let e = mesh_table(&cube, &PrimInspectorTab::Edge);
        assert_eq!(e.rows, 12);
        assert!(col(&e, "length").iter().all(|l| (l[0] - 1.0).abs() < 1e-6));
        // Polygons, not the triangles they are drawn with.
        let p = mesh_table(&cube, &PrimInspectorTab::Uniform);
        assert_eq!(p.rows, 6);
        assert!(col(&p, "corners").iter().all(|c| c[0] == 4.0));
        assert!(col(&p, "area").iter().all(|a| (a[0] - 1.0).abs() < 1e-6));
        assert_eq!(col(&p, "N")[3], vec![0.0, 1.0, 0.0]);
        // No UVs: nothing per corner. With UVs: one row per triangle corner.
        assert!(mesh_table(&cube, &PrimInspectorTab::FaceVarying).cols.is_empty());
        let mut grid = create_grid(2, 2, 2.0);
        grid.uvs = crate::core::uv::unwrap(&grid, crate::core::uv::UvMethod::Planar, 0.0, 0.0, 1, 1);
        let fv = mesh_table(&grid, &PrimInspectorTab::FaceVarying);
        assert_eq!(fv.rows, 24);
        assert_eq!(col(&fv, "uv").len(), 24);
        assert_eq!(col(&fv, "uv")[0].len(), 2);
    }

    #[test]
    fn a_clip_lists_joints_and_bones() {
        let clip = create_test_clip(1.0, FrameRate::new(30, 1));
        let j = clip_table(&clip, &PrimInspectorTab::Joint, 0);
        assert_eq!(j.rows, 19);
        let (heading, names) = j.labels.as_ref().unwrap();
        assert_eq!(heading, "joint");
        assert_eq!(names[0], "Take01:Hips");
        assert_eq!(col(&j, "parent")[0], vec![-1.0]);
        assert_eq!(col(&j, "parent")[1], vec![0.0]);
        assert!((col(&j, "P")[0][1] - 0.97).abs() < 1e-4);
        // The pose follows the frame.
        let later = clip_table(&clip, &PrimInspectorTab::Joint, 8);
        assert_ne!(col(&j, "R")[5], col(&later, "R")[5]);

        // A bone runs from a joint to the joint below it: 18 for 19 joints.
        let b = clip_table(&clip, &PrimInspectorTab::Bone, 0);
        assert_eq!(b.rows, 18);
        let (_, names) = b.labels.as_ref().unwrap();
        assert_eq!(names[0], "Take01:Hips > Take01:Spine");
        assert_eq!((col(&b, "from")[0][0], col(&b, "to")[0][0]), (0.0, 1.0));
        assert!((col(&b, "length")[0][0] - 0.12).abs() < 1e-4);
        assert!((col(&b, "dir")[0][1] - 1.0).abs() < 1e-3);
    }
}
