//! Ready-made graphs, one for each area of the program. Picking one from
//! the Templates menu replaces the current graph.

use bevy::math::Vec3;
use bevy_egui::egui;

use crate::core::poly::{ExtrudeMode, PolyMesh, PolyOp, PolyOpKind, PolySelection, SelSource, SubLevel};
use crate::examples;
use crate::ice::SubnetStore;
use crate::node_graph::NodeGraphState;
use crate::types::{NodeId, NodeType, RetimeMode, SubnetNodeType, DEFAULT_WRITE_PATH};

pub struct Template {
    pub group: &'static str,
    pub name:  &'static str,
    pub hint:  &'static str,
    /// Builds the graph and returns a line telling the user what to look at.
    pub build: fn(&mut NodeGraphState, &mut SubnetStore) -> String,
}

pub const GROUPS: [&str; 3] = ["Basics", "Modelling", "Animation & Mocap"];

pub const TEMPLATES: &[Template] = &[
    Template { group: "Basics", name: "Primitives", build: primitives,
        hint: "Cube, sphere and grid, each moved with a Transform and joined with Merge" },
    Template { group: "Basics", name: "Scatter and copy", build: scatter,
        hint: "Points scattered on a grid, a small cube copied onto each" },
    Template { group: "Basics", name: "ICE subnet", build: subnet,
        hint: "The same scatter and copy, built inside a subnet. Double-click the subnet node to open it" },
    Template { group: "Basics", name: "USD import", build: usd,
        hint: "Two example USD files loaded, moved and merged" },
    Template { group: "Modelling", name: "Edit Poly: tower", build: tower,
        hint: "Inset, extrude and bevel stacked on the top face of a cube" },
    Template { group: "Modelling", name: "Edit Poly: panels", build: panels,
        hint: "Every cell of a grid inset and extruded on its own" },
    Template { group: "Modelling", name: "Edit Poly: goblet", build: goblet,
        hint: "Edge loops, transforms and subdivision, with the early operations collapsed" },
    Template { group: "Modelling", name: "Edit Poly: bridge", build: bridge,
        hint: "Two cubes joined with Bridge, the selection made by a box rule" },
    Template { group: "Modelling", name: "Edit Poly: sea mine", build: sea_mine,
        hint: "Eleven operations with three rounds of subdivision: about ten thousand polygons from one cube" },
    Template { group: "Animation & Mocap", name: "Clip basics", build: clip_basics,
        hint: "Test clip renamed, trimmed, retimed and given a start timecode. Select each node to see the timeline follow" },
    Template { group: "Animation & Mocap", name: "FBX import", build: fbx_import,
        hint: "A real two-character mocap take loaded from FBX" },
    Template { group: "Animation & Mocap", name: "T-pose and export", build: tpose,
        hint: "Auto T-pose, a manual fix, proxy skin and Write FBX" },
    Template { group: "Animation & Mocap", name: "Mocap split (example takes)", build: mocap_example,
        hint: "The mocap split graph, pointed at the example folder with a two-character take" },
    Template { group: "Animation & Mocap", name: "Mocap split", build: mocap_blank,
        hint: "Folder of takes, split per character, animation and skinned T-pose written per character" },
];

// ── Helpers ──────────────────────────────────────────────────────────────────

fn p(x: f32, y: f32) -> egui::Pos2 { egui::pos2(x, y) }

fn output(graph: &NodeGraphState) -> NodeId {
    graph.nodes.iter().find(|n| matches!(n.node_type, NodeType::Output)).map(|n| n.id).expect("graph has an Output node")
}

/// Wire the last node to Output, park Output under it and select `focus`.
fn finish(graph: &mut NodeGraphState, last: NodeId, focus: NodeId, out_pos: egui::Pos2) {
    let out = output(graph);
    graph.add_connection(last, 0, out, 0);
    if let Some(n) = graph.nodes.iter_mut().find(|n| n.id == out) { n.position = out_pos; }
    graph.selected_node  = Some(focus);
    graph.selected_nodes = vec![focus];
}

/// View and select a clip node. Clips are not wired to Output.
fn view(graph: &mut NodeGraphState, node: NodeId, out_pos: egui::Pos2) {
    let out = output(graph);
    if let Some(n) = graph.nodes.iter_mut().find(|n| n.id == out) { n.position = out_pos; }
    graph.view_flag      = Some(node);
    graph.selected_node  = Some(node);
    graph.selected_nodes = vec![node];
}

fn transform(t: [f32; 3], r: [f32; 3], s: [f32; 3]) -> NodeType {
    NodeType::Transform { translation: Vec3::from_array(t), rotation: Vec3::from_array(r), scale: Vec3::from_array(s) }
}

fn edit_poly(ops: Vec<PolyOp>, pending: PolySelection) -> NodeType {
    NodeType::EditPoly { ops, pending, edit: None, auto_collapse: false }
}

fn picked(polys: Vec<u32>) -> PolySelection { PolySelection::picked_polys(polys) }

fn missing_examples() -> &'static str {
    if examples::dir().is_some() { "" } else { " The examples folder was not found: put it next to the program." }
}

// ── Basics ───────────────────────────────────────────────────────────────────

fn primitives(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let cube   = g.add_node("Cube".into(), NodeType::CreateCube { size: 1.0 }, p(40.0, 20.0));
    let sphere = g.add_node("Sphere".into(), NodeType::CreateSphere { radius: 0.6, segments: 24 }, p(250.0, 20.0));
    let grid   = g.add_node("Grid".into(), NodeType::CreateGrid { rows: 8, cols: 8, size: 5.0 }, p(460.0, 20.0));
    let cube_x = g.add_node("MoveCube".into(), transform([-1.2, 0.5, 0.0], [0.0, 30.0, 0.0], [1.0, 1.0, 1.0]), p(40.0, 120.0));
    let sph_x  = g.add_node("MoveSphere".into(), transform([1.2, 0.6, 0.0], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]), p(250.0, 120.0));
    let m1     = g.add_node("Merge".into(), NodeType::Merge, p(145.0, 220.0));
    let m2     = g.add_node("MergeFloor".into(), NodeType::Merge, p(300.0, 320.0));
    g.add_connection(cube, 0, cube_x, 0);
    g.add_connection(sphere, 0, sph_x, 0);
    g.add_connection(cube_x, 0, m1, 0);
    g.add_connection(sph_x, 0, m1, 1);
    g.add_connection(m1, 0, m2, 0);
    g.add_connection(grid, 0, m2, 1);
    finish(g, m2, cube_x, p(300.0, 420.0));
    "Primitives: select a Transform node and change its values in Properties.".into()
}

fn scatter(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let grid  = g.add_node("Ground".into(), NodeType::CreateGrid { rows: 10, cols: 10, size: 6.0 }, p(60.0, 20.0));
    let pts   = g.add_node("Scatter".into(), NodeType::ScatterPoints { count: 150, seed: 7 }, p(60.0, 120.0));
    let cube  = g.add_node("Pebble".into(), NodeType::CreateCube { size: 0.15 }, p(300.0, 20.0));
    let copy  = g.add_node("Copy".into(), NodeType::CopyToPoints, p(180.0, 220.0));
    let merge = g.add_node("Merge".into(), NodeType::Merge, p(180.0, 320.0));
    g.add_connection(grid, 0, pts, 0);
    g.add_connection(cube, 0, copy, 0);
    g.add_connection(pts, 0, copy, 1);
    g.add_connection(copy, 0, merge, 0);
    g.add_connection(grid, 0, merge, 1);
    finish(g, merge, pts, p(180.0, 420.0));
    "Scatter and copy: change Count or Seed on the Scatter node.".into()
}

fn subnet(g: &mut NodeGraphState, store: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let id = store.create_subnet("ScatterCopy".into());
    if let Some(sub) = store.get_mut(id) {
        // SubInput and SubOutput are made with the subnet.
        let (input, out) = (sub.nodes[0].id, sub.nodes[1].id);
        let pts  = sub.add_node("Scatter".into(), SubnetNodeType::ScatterPoints { count: 80, seed: 3 }, p(220.0, 140.0));
        let copy = sub.add_node("Copy".into(), SubnetNodeType::CopyToPoints, p(360.0, 220.0));
        sub.add_connection(input, 0, pts, 0);      // Mesh -> Geometry
        sub.add_connection(pts, 0, copy, 0);       // Points
        sub.add_connection(input, 2, copy, 1);     // Template
        sub.add_connection(copy, 0, out, 0);
    }
    let grid   = g.add_node("Ground".into(), NodeType::CreateGrid { rows: 6, cols: 6, size: 5.0 }, p(60.0, 20.0));
    let sphere = g.add_node("Ball".into(), NodeType::CreateSphere { radius: 0.12, segments: 10 }, p(300.0, 20.0));
    let node   = g.add_node("ScatterCopy".into(), NodeType::Subnet { id, name: "ScatterCopy".into() }, p(180.0, 140.0));
    let merge  = g.add_node("Merge".into(), NodeType::Merge, p(180.0, 260.0));
    g.add_connection(grid, 0, node, 0);
    g.add_connection(sphere, 0, node, 1);
    g.add_connection(node, 0, merge, 0);
    g.add_connection(grid, 0, merge, 1);
    finish(g, merge, node, p(180.0, 360.0));
    "ICE subnet: double-click the ScatterCopy node to see the graph inside it.".into()
}

fn usd(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let shapes = g.add_node("Shapes".into(), NodeType::LoadUsd { path: examples::path(examples::SHAPES_USD) }, p(40.0, 20.0));
    let table  = g.add_node("Table".into(), NodeType::LoadUsd { path: examples::path(examples::TABLE_USD) }, p(300.0, 20.0));
    let lift   = g.add_node("OnTable".into(), transform([0.0, 0.77, 0.0], [0.0, 0.0, 0.0], [0.35, 0.35, 0.35]), p(40.0, 120.0));
    let merge  = g.add_node("Merge".into(), NodeType::Merge, p(170.0, 220.0));
    g.add_connection(shapes, 0, lift, 0);
    g.add_connection(lift, 0, merge, 0);
    g.add_connection(table, 0, merge, 1);
    finish(g, merge, shapes, p(170.0, 320.0));
    format!("USD import: three shapes from one file, a table from another.{}", missing_examples())
}

// ── Modelling ────────────────────────────────────────────────────────────────

fn tower(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    // Polygon 3 is the top face of the cube; it keeps its index through
    // inset, extrude and bevel.
    let top = || picked(vec![3]);
    let group = ExtrudeMode::Group;
    let ops = vec![
        PolyOp::new(top(), PolyOpKind::Inset { amount: 0.12, by_polygon: false }),
        PolyOp::new(top(), PolyOpKind::Extrude { height: 0.7, mode: group }),
        PolyOp::new(top(), PolyOpKind::Bevel { height: 0.15, outline: 0.12, mode: group }),
        PolyOp::new(top(), PolyOpKind::Extrude { height: 0.5, mode: group }),
        PolyOp::new(top(), PolyOpKind::Bevel { height: 0.45, outline: -0.3, mode: group }),
        PolyOp::new(top(), PolyOpKind::Inset { amount: 0.05, by_polygon: false }),
        PolyOp::new(top(), PolyOpKind::Extrude { height: -0.2, mode: group }),
    ];
    let cube = g.add_node("Cube".into(), NodeType::CreateCube { size: 1.0 }, p(180.0, 20.0));
    let ep   = g.add_node("Tower".into(), edit_poly(ops, top()), p(180.0, 130.0));
    g.add_connection(cube, 0, ep, 0);
    finish(g, ep, ep, p(180.0, 260.0));
    "Edit Poly tower: seven live operations on the top face. Change their values in Properties.".into()
}

fn panels(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    // The 36 cells keep their indices; new polygons are added after them.
    let cells = || picked((0..36).collect());
    let every_other = || picked((0..36).filter(|c| (c / 6 + c % 6) % 2 == 0).collect());
    let ops = vec![
        PolyOp::new(cells(), PolyOpKind::Inset { amount: 0.05, by_polygon: true }),
        PolyOp::new(cells(), PolyOpKind::Extrude { height: 0.08, mode: ExtrudeMode::ByPolygon }),
        PolyOp::new(every_other(), PolyOpKind::Bevel { height: 0.25, outline: -0.08, mode: ExtrudeMode::ByPolygon }),
    ];
    let grid = g.add_node("Grid".into(), NodeType::CreateGrid { rows: 6, cols: 6, size: 3.0 }, p(180.0, 20.0));
    let ep   = g.add_node("Panels".into(), edit_poly(ops, every_other()), p(180.0, 130.0));
    g.add_connection(grid, 0, ep, 0);
    finish(g, ep, ep, p(180.0, 260.0));
    "Edit Poly panels: by-polygon inset and extrude on every cell, a bevel on every other one.".into()
}

fn goblet(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let cube = PolyMesh::from_mesh(&crate::node_graph::nodes::create_cube(1.0));
    // The four upright edges: one of them, grown into its ring.
    let up = cube.edges().into_iter()
        .find(|e| (cube.verts[e[0] as usize] - cube.verts[e[1] as usize]).y.abs() > 0.9)
        .unwrap_or([0, 1]);
    let mut ring = PolySelection { level: SubLevel::Edge, edges: vec![up], ..Default::default() };
    ring.expand_edges(&cube, true);
    // Connect renumbers the polygons: find the top face on the mesh it
    // leaves. From there on the face keeps its index.
    let connect = PolyOp::new(ring.clone(), PolyOpKind::Connect { segments: 2 });
    let mut cut = cube.clone();
    connect.apply(&mut cut);
    let top_index = (0..cut.polys.len()).find(|q| cut.normal(*q).y > 0.9).unwrap_or(0) as u32;
    let top = || picked(vec![top_index]);
    let group = ExtrudeMode::Group;
    // Foot and stem, collapsed.
    let mut ops = vec![
        connect,
        PolyOp::new(top(), PolyOpKind::Transform { translate: [0.0; 3], rotate: [0.0, 0.0, 0.0, 1.0], scale: [0.3, 1.0, 0.3] }),
        PolyOp::new(top(), PolyOpKind::Extrude { height: 0.7, mode: group }),
    ];
    crate::core::poly::collapse_all(&mut ops);
    // The cup, live.
    ops.push(PolyOp::new(top(), PolyOpKind::Bevel { height: 0.6, outline: 0.5, mode: group }));
    ops.push(PolyOp::new(top(), PolyOpKind::Extrude { height: 0.5, mode: group }));
    ops.push(PolyOp::new(top(), PolyOpKind::Inset { amount: 0.1, by_polygon: false }));
    ops.push(PolyOp::new(top(), PolyOpKind::Extrude { height: -0.9, mode: group }));
    ops.push(PolyOp::new(PolySelection { source: SelSource::All, ..Default::default() }, PolyOpKind::Subdivide { iterations: 2 }));
    let node = g.add_node("Cube".into(), NodeType::CreateCube { size: 1.0 }, p(180.0, 20.0));
    let ep   = g.add_node("Goblet".into(), edit_poly(ops, top()), p(180.0, 130.0));
    g.add_connection(node, 0, ep, 0);
    finish(g, ep, ep, p(180.0, 260.0));
    "Edit Poly goblet: three collapsed operations for the foot and stem, five live ones for the cup. Restore brings the collapsed ones back.".into()
}

fn bridge(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    // The two faces that look at each other: their centres lie in this box.
    let facing = PolySelection {
        source: SelSource::InBox { min: [-0.1, 0.4, -0.1], max: [0.1, 2.1, 0.1] },
        ..Default::default()
    };
    let ops = vec![
        PolyOp::new(facing, PolyOpKind::Bridge),
        PolyOp::new(PolySelection { level: SubLevel::Edge, source: SelSource::InBox { min: [-0.6, 0.4, -0.6], max: [0.6, 2.1, 0.6] }, ..Default::default() },
            PolyOpKind::Connect { segments: 3 }),
    ];
    let a    = g.add_node("Lower".into(), NodeType::CreateCube { size: 1.0 }, p(60.0, 20.0));
    let b    = g.add_node("Upper".into(), NodeType::CreateCube { size: 1.0 }, p(300.0, 20.0));
    let lift = g.add_node("Lift".into(), transform([0.0, 2.5, 0.0], [0.0, 45.0, 0.0], [1.0, 1.0, 1.0]), p(300.0, 120.0));
    let m    = g.add_node("Merge".into(), NodeType::Merge, p(180.0, 220.0));
    let ep   = g.add_node("Bridge".into(), edit_poly(ops, Default::default()), p(180.0, 320.0));
    g.add_connection(b, 0, lift, 0);
    g.add_connection(a, 0, m, 0);
    g.add_connection(lift, 0, m, 1);
    g.add_connection(m, 0, ep, 0);
    finish(g, ep, ep, p(180.0, 440.0));
    "Edit Poly bridge: the selection is a box rule, so it still works when the Lift node turns the upper cube.".into()
}

fn sea_mine(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let all = || PolySelection { source: SelSource::All, ..Default::default() };
    // One round of subdivision turns the cube into 24 quads. They keep the
    // indices 0 to 23 through every inset, extrude and bevel that follows.
    let plates = || picked((0..24).collect());
    let long   = || picked((0..24).filter(|q| q % 2 == 0).collect());
    let short  = || picked((0..24).filter(|q| q % 2 == 1).collect());
    let each = ExtrudeMode::ByPolygon;
    // The hull, collapsed.
    let mut ops = vec![
        PolyOp::new(all(), PolyOpKind::Subdivide { iterations: 1 }),
        PolyOp::new(all(), PolyOpKind::Transform { translate: [0.0, 1.2, 0.0], rotate: [0.0, 0.0, 0.0, 1.0], scale: [2.4, 2.4, 2.4] }),
        PolyOp::new(plates(), PolyOpKind::Inset { amount: 0.06, by_polygon: true }),
        PolyOp::new(plates(), PolyOpKind::Extrude { height: -0.05, mode: each }),
        PolyOp::new(plates(), PolyOpKind::Inset { amount: 0.07, by_polygon: true }),
    ];
    crate::core::poly::collapse_all(&mut ops);
    // Horns and ports, live.
    ops.push(PolyOp::new(plates(), PolyOpKind::Bevel { height: 0.12, outline: -0.05, mode: each }));
    ops.push(PolyOp::new(long(), PolyOpKind::Bevel { height: 0.55, outline: -0.07, mode: each }));
    ops.push(PolyOp::new(long(), PolyOpKind::Bevel { height: 0.06, outline: 0.05, mode: each }));
    ops.push(PolyOp::new(short(), PolyOpKind::Inset { amount: 0.04, by_polygon: true }));
    ops.push(PolyOp::new(short(), PolyOpKind::Extrude { height: -0.12, mode: each }));
    ops.push(PolyOp::new(all(), PolyOpKind::Subdivide { iterations: 2 }));
    let cube = g.add_node("Cube".into(), NodeType::CreateCube { size: 1.0 }, p(180.0, 20.0));
    let ep   = g.add_node("SeaMine".into(), edit_poly(ops, long()), p(180.0, 130.0));
    g.add_connection(cube, 0, ep, 0);
    finish(g, ep, ep, p(180.0, 260.0));
    "Edit Poly sea mine: about ten thousand polygons. Change the height of the horns in operation 7, or lower the last Subdivide to see the cage.".into()
}

// ── Animation & mocap ────────────────────────────────────────────────────────

fn clip_basics(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let x = 180.0;
    let clip   = g.add_node("TestClip".into(), NodeType::TestClip { seconds: 4.0, fps_num: 30, fps_den: 1 }, p(x, 20.0));
    let rename = g.add_node("Rename".into(), NodeType::RenameJoints {
        find: "Left".into(), replace: "L_".into(), strip_namespace: true, prefix: "hero_".into() }, p(x, 110.0));
    let trim   = g.add_node("Trim".into(), NodeType::TrimClip { head: 15, tail: 15 }, p(x, 200.0));
    let retime = g.add_node("To24fps".into(), NodeType::Retime { fps_num: 24, fps_den: 1, mode: RetimeMode::Resample }, p(x, 290.0));
    let tc     = g.add_node("Timecode".into(), NodeType::SetTimecode { hours: 10, minutes: 0, seconds: 0, frames: 0, drop_frame: false }, p(x, 380.0));
    g.add_connection(clip, 0, rename, 0);
    g.add_connection(rename, 0, trim, 0);
    g.add_connection(trim, 0, retime, 0);
    g.add_connection(retime, 0, tc, 0);
    view(g, tc, p(x, 490.0));
    g.selected_node  = Some(trim);
    g.selected_nodes = vec![trim];
    "Clip basics: select each node in turn. The timeline shows that node's range, rate and timecode. Space plays.".into()
}

fn fbx_import(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let load = g.add_node("Take".into(), NodeType::LoadFbx { path: examples::path(examples::TAKE_FBX), take: 0 }, p(180.0, 20.0));
    let trim = g.add_node("Trim".into(), NodeType::TrimClip { head: 120, tail: 120 }, p(180.0, 120.0));
    g.add_connection(load, 0, trim, 0);
    view(g, trim, p(180.0, 240.0));
    format!("FBX import: a motion capture take of two people, 120 fps, a second trimmed off each end. Space plays.{}", missing_examples())
}

fn tpose(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let x = 180.0;
    let clip  = g.add_node("TestClip".into(), NodeType::TestClip { seconds: 2.0, fps_num: 30, fps_den: 1 }, p(x, 20.0));
    let pose  = g.add_node("TPose".into(), NodeType::AutoTPose { set_hip_height: true, hip_height: 95.0 }, p(x, 110.0));
    let arm = |joint: &str, z: f32| crate::core::anim::PoseEdit { joint: joint.into(), rotation: [0.0, 0.0, z], translation: [0.0; 3] };
    let fix   = g.add_node("FixPose".into(), NodeType::FixPose {
        edits: vec![arm("Take01:LeftArm", -20.0), arm("Take01:RightArm", 20.0)] }, p(x, 200.0));
    let skin  = g.add_node("ProxySkin".into(), NodeType::ProxySkin { thickness: 1.6 }, p(x, 290.0));
    let write = g.add_node("WriteTPose".into(), NodeType::WriteFbx { path: DEFAULT_WRITE_PATH.replace(".fbx", "_tpose.fbx") }, p(x, 380.0));
    g.add_connection(clip, 0, pose, 0);
    g.add_connection(pose, 0, fix, 0);
    g.add_connection(fix, 0, skin, 0);
    g.add_connection(skin, 0, write, 0);
    view(g, skin, p(x, 490.0));
    g.selected_node  = Some(fix);
    g.selected_nodes = vec![fix];
    "T-pose and export: the Fix Pose node lowers both arms. Set a path on WriteTPose and press Write to export.".into()
}

fn mocap_example(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::mocap_split_template(g);
    let dir = examples::path(examples::TAKES_DIR);
    for n in &mut g.nodes {
        if let NodeType::LoadFbxDir { dir: d, .. } = &mut n.node_type { *d = dir.clone(); }
    }
    format!("Mocap split on the example take: two characters, split into one clip each.{}", missing_examples())
}

fn mocap_blank(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::mocap_split_template(g);
    "Template loaded. Select the Takes node and choose a folder.".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MeshData, SubnetId};

    #[test]
    fn every_template_builds_something_to_look_at() {
        // The file-based templates need the examples folder of the repository.
        assert!(examples::dir().is_some(), "examples folder not found");

        for t in TEMPLATES {
            assert!(GROUPS.contains(&t.group), "{}", t.name);
            let mut g = NodeGraphState::default();
            let mut store = SubnetStore::default();
            // Twice: a template has to replace whatever was there.
            (t.build)(&mut g, &mut store);
            let message = (t.build)(&mut g, &mut store);
            assert!(!message.is_empty() && !message.contains("not found"), "{}: {message}", t.name);
            assert_eq!(g.nodes.iter().filter(|n| matches!(n.node_type, NodeType::Output)).count(), 1);

            let eval = |sid: SubnetId, mesh: &MeshData, template: Option<&MeshData>| -> MeshData {
                store.get(sid).map(|sg| sg.evaluate(mesh, template)).unwrap_or_else(|| mesh.clone())
            };
            // Every node cooks: a clip or a mesh with vertices.
            for n in &g.nodes {
                // The blank mocap template has no folder yet.
                if t.name == "Mocap split" { break; }
                if matches!(n.node_type, NodeType::Output | NodeType::WriteFbx { .. }) { continue; }
                let clip = (0..n.outputs.len().max(1)).any(|o| g.eval_anim_out(n.id, o).is_some());
                let mesh = g.eval_node(n.id, &mut std::collections::HashMap::new(), &eval)
                    .map(|r| r.into_mesh()).map(|m| m.vertices.len() + m.points.len()).unwrap_or(0);
                assert!(clip || mesh > 0, "{}: node {} gives nothing", t.name, n.name);
            }
            // Mesh templates end in Output; clip templates set the view flag.
            let out = output(&g);
            let wired = g.connections.iter().any(|c| c.to_node == out);
            assert!(wired || g.view_flag.is_some(), "{}", t.name);
            assert!(g.selected_node.is_some(), "{}", t.name);
        }
    }

    #[test]
    fn modelling_templates_give_valid_meshes() {
        for (name, verts_at_least) in [("Edit Poly: tower", 30), ("Edit Poly: panels", 300), ("Edit Poly: goblet", 300), ("Edit Poly: bridge", 20), ("Edit Poly: sea mine", 8000)] {
            let t = TEMPLATES.iter().find(|t| t.name == name).unwrap();
            let mut g = NodeGraphState::default();
            (t.build)(&mut g, &mut SubnetStore::default());
            let ep = g.nodes.iter().find(|n| matches!(n.node_type, NodeType::EditPoly { .. })).unwrap().id;
            let eval = |_: SubnetId, mesh: &MeshData, _: Option<&MeshData>| mesh.clone();
            let mesh = g.eval_node(ep, &mut std::collections::HashMap::new(), &eval).unwrap().into_mesh();
            let poly = PolyMesh::from_mesh(&mesh);
            assert!(poly.verts.len() >= verts_at_least, "{name}: {} vertices", poly.verts.len());
            for q in &poly.polys {
                assert!(q.len() >= 3);
                assert_eq!(q.iter().collect::<std::collections::HashSet<_>>().len(), q.len(), "{name}");
            }
            if name != "Edit Poly: panels" { assert!(poly.is_closed(), "{name} is open"); }
            if name == "Edit Poly: goblet" {
                // It stands upright: taller than wide, centred on the axis.
                let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for v in &poly.verts { lo = lo.min(*v); hi = hi.max(*v); }
                assert!(hi.y - lo.y > 1.8 && hi.y - lo.y > hi.x - lo.x, "{lo} {hi}");
                assert!((lo.x + hi.x).abs() < 0.05 && (lo.z + hi.z).abs() < 0.05, "{lo} {hi}");
            }
        }
    }
}
