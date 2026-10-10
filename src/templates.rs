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
    /// Whose work the template shows: the first level of the Templates menu.
    pub by:    &'static str,
    pub group: &'static str,
    pub name:  &'static str,
    pub hint:  &'static str,
    /// Builds the graph and returns a line telling the user what to look at.
    pub build: fn(&mut NodeGraphState, &mut SubnetStore) -> String,
}

pub const MARTINO: &str = "Martino";
pub const SIMON:   &str = "Simon";

/// The Templates menu: whose work, then the groups of each.
pub const MENU: [(&str, &[&str]); 2] = [
    (MARTINO, &["USD stages", "ICE"]),
    (SIMON,   &["Basics", "Modelling", "UV", "USD", "Animation & Mocap"]),
];

pub const TEMPLATES: &[Template] = &[
    Template { by: MARTINO, group: "USD stages", name: "Composition: a street", build: street,
        hint: "A stage composed from four files: a sublayer, references, a variant set and an override in the root layer" },
    Template { by: MARTINO, group: "USD stages", name: "Curves and points", build: strands,
        hint: "BasisCurves of every basis (linear, Bezier, Catmull-Rom, a periodic B-spline), 600 strands of grass and a spiral of points of growing width" },
    Template { by: MARTINO, group: "USD stages", name: "Point instancer: a forest", build: forest,
        hint: "4,900 pines and stones from one PointInstancer, drawn by GPU instancing. The Transform after it moves the placements, not the shared meshes" },
    Template { by: MARTINO, group: "USD stages", name: "Native instances: a hall of tables", build: hall,
        hint: "64 instanceable references to one table: five meshes, shared by every table" },
    Template { by: MARTINO, group: "USD stages", name: "Purposes: street lamps", build: lamps,
        hint: "Lamps with render, proxy and guide geometry. Switch them with the render, proxy and guide toggles of the Scene Explorer" },
    Template { by: MARTINO, group: "ICE", name: "ICE tree: spherify", build: spherify,
        hint: "A subdivided cube pushed onto a sphere by an ICE tree. Select the ICE node and press the down arrow to go inside, the up arrow to come back" },
    Template { by: SIMON, group: "Basics", name: "Primitives", build: primitives,
        hint: "Cube, sphere and grid, each moved with a Transform and joined with Merge" },
    Template { by: SIMON, group: "Basics", name: "Scatter and copy", build: scatter,
        hint: "Points scattered on a grid, a small cube copied onto each" },
    Template { by: SIMON, group: "Basics", name: "ICE subnet", build: subnet,
        hint: "The same scatter and copy, built inside a subnet. Double-click the subnet node to open it" },
    Template { by: SIMON, group: "Basics", name: "USD import", build: usd,
        hint: "Two example USD files loaded, moved and merged" },
    Template { by: SIMON, group: "Modelling", name: "Edit Poly: tower", build: tower,
        hint: "Inset, extrude and bevel stacked on the top face of a cube" },
    Template { by: SIMON, group: "Modelling", name: "Edit Poly: panels", build: panels,
        hint: "Every cell of a grid inset and extruded on its own" },
    Template { by: SIMON, group: "Modelling", name: "Edit Poly: goblet", build: goblet,
        hint: "Edge loops, transforms and subdivision, with the early operations collapsed" },
    Template { by: SIMON, group: "Modelling", name: "Edit Poly: bridge", build: bridge,
        hint: "Two cubes joined with Bridge, the selection made by a box rule" },
    Template { by: SIMON, group: "Modelling", name: "Edit Poly: sea mine", build: sea_mine,
        hint: "Eleven operations with three rounds of subdivision: about ten thousand polygons from one cube" },
    Template { by: SIMON, group: "Modelling", name: "Edit Poly: bolt", build: bolt,
        hint: "Chamfer, slice, hinge, outline and vertex extrude: the second set of Edit Poly operations" },
    Template { by: SIMON, group: "UV", name: "Unwrap the sea mine", build: uv_mine,
        hint: "Conformal (LSCM) unwrap of ten thousand polygons. Open the UV Editor pane to see the charts" },
    Template { by: SIMON, group: "UV", name: "Edit UV islands", build: uv_edit,
        hint: "A box unwrapped, then islands moved, turned and scaled in a UV Edit node" },
    Template { by: SIMON, group: "UV", name: "UDIM tiles", build: uv_udim,
        hint: "A body and two hands made of separate pieces, unwrapped over three UDIM tiles: each hand keeps its fingers on its own tile" },
    Template { by: SIMON, group: "USD", name: "USD: heavy model", build: usd_heavy,
        hint: "A 460,000 triangle model as 43 packed primitives. Select the Load USD node for its materials and textures; open the UV Editor for its UVs" },
    Template { by: SIMON, group: "USD", name: "USD: pick and edit", build: usd_pick,
        hint: "One wheel picked out of the model and moved. Only the picked primitive goes through the Transform and Edit Poly nodes" },
    Template { by: SIMON, group: "USD", name: "USD: prune", build: usd_prune,
        hint: "The model cut down to its wheels and callipers with a path pattern" },
    Template { by: SIMON, group: "Animation & Mocap", name: "Clip basics", build: clip_basics,
        hint: "Test clip renamed, trimmed, retimed and given a start timecode. Select each node to see the timeline follow" },
    Template { by: SIMON, group: "Animation & Mocap", name: "FBX import", build: fbx_import,
        hint: "A real two-character mocap take loaded from FBX" },
    Template { by: SIMON, group: "Animation & Mocap", name: "T-pose and export", build: tpose,
        hint: "Auto T-pose, a manual fix, proxy skin and Write FBX" },
    Template { by: SIMON, group: "Animation & Mocap", name: "Mocap tools", build: mocap_tools,
        hint: "The example take split, pruned, smoothed, held in place, mirrored and looped" },
    Template { by: SIMON, group: "Animation & Mocap", name: "Retarget", build: retarget,
        hint: "A character of the example take driving the test skeleton, which rests in another pose" },
    Template { by: SIMON, group: "Animation & Mocap", name: "Body Collide: into the car", build: body_collide,
        hint: "A captured actor walks through a car and sits in it for six minutes, kept out of the seat, the floor and himself. Opens solved" },
    Template { by: SIMON, group: "Animation & Mocap", name: "Mocap split (example takes)", build: mocap_example,
        hint: "The mocap split graph, pointed at the example folder with a two-character take" },
    Template { by: SIMON, group: "Animation & Mocap", name: "Mocap split", build: mocap_blank,
        hint: "Folder of takes, split per character, animation and skinned T-pose written per character" },
];

impl Template {
    /// Build the template, lay its graph out and bring it into view.
    pub fn load(&self, graph: &mut NodeGraphState, subnets: &mut SubnetStore) -> String {
        let message = (self.build)(graph, subnets);
        let (selected, many) = (graph.selected_node, graph.selected_nodes.clone());
        graph.auto_layout();
        graph.selected_node = selected;
        graph.selected_nodes = many;
        message
    }
}

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
        PolyOp::new(top(), PolyOpKind::Transform { translate: [0.0; 3], rotate: [0.0, 0.0, 0.0, 1.0], scale: [0.3, 1.0, 0.3], falloff: 0.0 }),
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
        PolyOp::new(all(), PolyOpKind::Transform { translate: [0.0, 1.2, 0.0], rotate: [0.0, 0.0, 0.0, 1.0], scale: [2.4, 2.4, 2.4], falloff: 0.0 }),
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

fn bolt(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let top = || picked(vec![3]);
    let all_edges = PolySelection { level: SubLevel::Edge, source: SelSource::All, ..Default::default() };
    let top_corners = PolySelection { level: SubLevel::Vertex, source: SelSource::InBox { min: [-2.0, 1.7, -2.0], max: [2.0, 3.0, 2.0] }, ..Default::default() };
    let ops = vec![
        // Head: a slab, its edges taken off.
        PolyOp::new(PolySelection { source: SelSource::All, ..Default::default() },
            PolyOpKind::Transform { translate: [0.0, 0.2, 0.0], rotate: [0.0, 0.0, 0.0, 1.0], scale: [1.6, 0.4, 1.6], falloff: 0.0 }),
        PolyOp::new(all_edges, PolyOpKind::Chamfer { amount: 0.06 }),
        // Shank: inset the top, pull it up, cut it into rings.
        PolyOp::new(top(), PolyOpKind::Inset { amount: 0.35, by_polygon: false }),
        PolyOp::new(top(), PolyOpKind::Extrude { height: 1.5, mode: ExtrudeMode::Group }),
        PolyOp::new(Default::default(), PolyOpKind::Slice { axis: 1, offset: 0.8 }),
        PolyOp::new(Default::default(), PolyOpKind::Slice { axis: 1, offset: 1.2 }),
        PolyOp::new(Default::default(), PolyOpKind::Slice { axis: 1, offset: 1.6 }),
        // Tip: narrower, a spike on each corner, and a lid hinged open.
        PolyOp::new(top(), PolyOpKind::Outline { amount: -0.08 }),
        PolyOp::new(top_corners, PolyOpKind::ExtrudeVertex { height: 0.15, width: 0.08 }),
        PolyOp::new(top(), PolyOpKind::Hinge { angle: 50.0, segments: 5, edge: 0 }),
    ];
    let cube = g.add_node("Cube".into(), NodeType::CreateCube { size: 1.0 }, p(180.0, 20.0));
    let ep   = g.add_node("Bolt".into(), edit_poly(ops, top()), p(180.0, 130.0));
    g.add_connection(cube, 0, ep, 0);
    finish(g, ep, ep, p(180.0, 260.0));
    "Edit Poly bolt: chamfer, slice, outline, vertex extrude and hinge. Change the hinge angle in the last operation.".into()
}

// ── USD ──────────────────────────────────────────────────────────────────────

fn load_delorean(g: &mut NodeGraphState) -> NodeId {
    crate::graph_io::clear(g);
    g.add_node("DeLorean".into(), NodeType::LoadUsd { path: examples::path(examples::DELOREAN_USD) }, p(180.0, 20.0))
}

fn usd_heavy(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    let load = load_delorean(g);
    finish(g, load, load, p(180.0, 130.0));
    format!("USD: 43 packed primitives, 461,595 triangles. The Properties of the Load USD node list its materials and textures. The UV Editor tab shows its UVs.{}", missing_examples())
}

fn usd_pick(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    let load = load_delorean(g);
    let pick = g.add_node("PickWheel".into(), NodeType::PickPrims { pattern: "Wheel_Front_L".into() }, p(180.0, 110.0));
    let lift = g.add_node("PullOut".into(), transform([0.6, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]), p(180.0, 200.0));
    let edit = g.add_node("EditWheel".into(), edit_poly(vec![], PolySelection::default()), p(180.0, 290.0));
    g.add_connection(load, 0, pick, 0);
    g.add_connection(pick, 0, lift, 0);
    g.add_connection(lift, 0, edit, 0);
    finish(g, edit, pick, p(180.0, 380.0));
    format!("USD pick: the front left wheel is picked by a path pattern, pulled out by the Transform, and is what the Edit Poly node edits. The other 42 primitives pass through untouched.{}", missing_examples())
}

fn usd_prune(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    let load = load_delorean(g);
    let prune = g.add_node("WheelsOnly".into(), NodeType::PrunePrims { pattern: "Wheel, Calliper".into(), keep: true }, p(180.0, 110.0));
    g.add_connection(load, 0, prune, 0);
    finish(g, prune, prune, p(180.0, 200.0));
    format!("USD prune: only the primitives whose path matches the pattern are kept. Switch to \"Remove the matches\" for the car without its wheels.{}", missing_examples())
}

// ── UV ───────────────────────────────────────────────────────────────────────

fn uv_mine(g: &mut NodeGraphState, store: &mut SubnetStore) -> String {
    sea_mine(g, store);
    let ep = g.nodes.iter().find(|n| matches!(n.node_type, NodeType::EditPoly { .. })).map(|n| n.id).expect("sea mine has an Edit Poly node");
    let uv = g.add_node("Unwrap".into(), NodeType::UvUnwrap {
        method: crate::core::uv::UvMethod::Conformal, angle: 50.0, margin: 0.01, axis: 1, tiles: 1 }, p(180.0, 240.0));
    g.add_connection(ep, 0, uv, 0);
    finish(g, uv, uv, p(180.0, 350.0));
    "UV unwrap: open the UV Editor pane from the Panes menu. Lower the chart angle for more charts with less distortion.".into()
}

fn uv_udim(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let cube   = g.add_node("Box".into(), NodeType::CreateCube { size: 1.0 }, p(260.0, 20.0));
    let sphere = g.add_node("Ball".into(), NodeType::CreateSphere { radius: 0.5, segments: 16 }, p(40.0, 20.0));
    let body   = g.add_node("Body".into(), transform([0.0, 1.5, 0.0], [0.0; 3], [1.6, 3.0, 1.0]), p(480.0, 110.0));
    g.add_connection(cube, 0, body, 0);
    let mut last = body;
    // Two hands: a palm with three fingers beside it, well away from the body.
    for (h, side) in [-1.0f32, 1.0].into_iter().enumerate() {
        let x = side * 3.0;
        let col = 40.0 + h as f32 * 220.0;
        let palm = g.add_node(format!("Palm{}", h + 1), transform([x, 1.5, 0.0], [0.0; 3], [1.0, 1.0, 0.5]), p(col, 110.0));
        g.add_connection(sphere, 0, palm, 0);
        let mut hand = palm;
        for f in 0..3 {
            let row = 200.0 + f as f32 * 90.0;
            let finger = g.add_node(format!("Finger{}_{}", h + 1, f + 1),
                transform([x - 0.3 + f as f32 * 0.3, 2.35, 0.0], [0.0; 3], [0.2, 0.7, 0.2]), p(col + 110.0, row));
            let merge = g.add_node(format!("Hand{}_{}", h + 1, f + 1), NodeType::Merge, p(col, row + 45.0));
            g.add_connection(cube, 0, finger, 0);
            g.add_connection(hand, 0, merge, 0);
            g.add_connection(finger, 0, merge, 1);
            hand = merge;
        }
        let join = g.add_node(format!("Join{}", h + 1), NodeType::Merge, p(480.0, 300.0 + h as f32 * 100.0));
        g.add_connection(last, 0, join, 0);
        g.add_connection(hand, 0, join, 1);
        last = join;
    }
    let uv = g.add_node("Unwrap".into(), NodeType::UvUnwrap {
        method: crate::core::uv::UvMethod::Conformal, angle: 60.0, margin: 0.02, axis: 1, tiles: 3 }, p(480.0, 500.0));
    g.add_connection(last, 0, uv, 0);
    finish(g, uv, uv, p(480.0, 590.0));
    "UDIM tiles: nine separate pieces over three tiles. The body has the most surface and takes 1001. Each hand keeps its palm and fingers together. Change the tile count on the Unwrap node.".into()
}

fn uv_edit(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    use crate::core::uv::{IslandEdit, UvMethod};
    crate::graph_io::clear(g);
    let cube = g.add_node("Box".into(), NodeType::CreateCube { size: 1.0 }, p(180.0, 20.0));
    let size = g.add_node("Stretch".into(), transform([0.0, 0.5, 0.0], [0.0, 0.0, 0.0], [2.0, 1.0, 1.0]), p(180.0, 110.0));
    let uv   = g.add_node("Unwrap".into(), NodeType::UvUnwrap { method: UvMethod::Conformal, angle: 45.0, margin: 0.03, axis: 1, tiles: 1 }, p(180.0, 200.0));
    let edit = g.add_node("Arrange".into(), NodeType::UvEdit { edits: vec![
        IslandEdit { island: 0, offset: [0.0, 0.0], rotate: 90.0, scale: [1.0, 1.0] },
        IslandEdit { island: 3, offset: [0.05, 0.1], rotate: 0.0, scale: [0.6, 0.6] },
    ] }, p(180.0, 290.0));
    g.add_connection(cube, 0, size, 0);
    g.add_connection(size, 0, uv, 0);
    g.add_connection(uv, 0, edit, 0);
    finish(g, edit, edit, p(180.0, 400.0));
    "UV edit: open the UV Editor pane. Click an island to select it, drag to move it; its values are in Properties.".into()
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
    let write = g.add_node("WriteTPose".into(), NodeType::WriteFbx { path: DEFAULT_WRITE_PATH.replace(".fbx", "_tpose.fbx"), mesh: true }, p(x, 380.0));
    g.add_connection(clip, 0, pose, 0);
    g.add_connection(pose, 0, fix, 0);
    g.add_connection(fix, 0, skin, 0);
    g.add_connection(skin, 0, write, 0);
    view(g, skin, p(x, 490.0));
    g.selected_node  = Some(fix);
    g.selected_nodes = vec![fix];
    "T-pose and export: the Fix Pose node lowers both arms. Set a path on WriteTPose and press Write to export.".into()
}

fn mocap_tools(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let x = 180.0;
    let load   = g.add_node("Take".into(), NodeType::LoadFbx { path: examples::path(examples::TAKE_FBX), take: 0 }, p(x, 20.0));
    let split  = g.add_node("OneCharacter".into(), NodeType::SplitSkeleton { picks: vec![crate::types::SplitPick::Character(0)] }, p(x, 100.0));
    let prune  = g.add_node("NoFingers".into(), NodeType::PruneJoints { words: "thumb, index, middle, ring, pinky, end".into() }, p(x, 180.0));
    let trim   = g.add_node("Trim".into(), NodeType::TrimClip { head: 120, tail: 240 }, p(x, 260.0));
    let smooth = g.add_node("Smooth".into(), NodeType::SmoothClip { radius: 4, amount: 1.0, translations: true }, p(x, 340.0));
    let place  = g.add_node("InPlace".into(), NodeType::InPlace { keep_height: true, to_root: false }, p(x, 420.0));
    let floor  = g.add_node("Floor".into(), NodeType::FloorClip { height: 0.0 }, p(x, 500.0));
    let mirror = g.add_node("Mirror".into(), NodeType::MirrorClip, p(x, 580.0));
    let lp     = g.add_node("Loop".into(), NodeType::LoopClip { blend: 30 }, p(x, 660.0));
    for (a, b) in [(load, split), (split, prune), (prune, trim), (trim, smooth), (smooth, place), (place, floor), (floor, mirror), (mirror, lp)] {
        g.add_connection(a, 0, b, 0);
    }
    view(g, lp, p(x, 760.0));
    g.selected_node  = Some(smooth);
    g.selected_nodes = vec![smooth];
    format!("Mocap tools: one node per step. Put the view flag on any of them to see the clip at that point.{}", missing_examples())
}

fn retarget(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let load  = g.add_node("Take".into(), NodeType::LoadFbx { path: examples::path(examples::TAKE_FBX), take: 0 }, p(40.0, 20.0));
    let split = g.add_node("OneCharacter".into(), NodeType::SplitSkeleton { picks: vec![crate::types::SplitPick::Character(0)] }, p(40.0, 110.0));
    let skel  = g.add_node("TargetSkeleton".into(), NodeType::TestClip { seconds: 1.0, fps_num: 30, fps_den: 1 }, p(300.0, 110.0));
    let ret   = g.add_node("Retarget".into(), NodeType::Retarget, p(170.0, 220.0));
    let skin  = g.add_node("ProxySkin".into(), NodeType::ProxySkin { thickness: 1.6 }, p(170.0, 320.0));
    g.add_connection(load, 0, split, 0);
    g.add_connection(split, 0, ret, 0);
    g.add_connection(skel, 0, ret, 1);
    g.add_connection(ret, 0, skin, 0);
    view(g, skin, p(170.0, 430.0));
    g.selected_node  = Some(ret);
    g.selected_nodes = vec![ret];
    format!("Retarget: a captured performance on the 19-joint test skeleton, which rests with its arms down. Space plays.{}", missing_examples())
}

fn body_collide(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let take  = g.add_node("Take".into(), NodeType::LoadFbx { path: examples::path(examples::RAGDOLL_TAKE), take: 0 }, p(40.0, 20.0));
    // Characterize before Car: side by side above Body Collide, in the order of its inputs.
    let who   = g.add_node("Characterize".into(), NodeType::Characterize { picks: vec![] }, p(40.0, 85.0));
    let car   = g.add_node("Car".into(), NodeType::LoadFbxMesh { path: examples::path(examples::RAGDOLL_SET) }, p(300.0, 20.0));
    let body  = g.add_node("BodyCollide".into(), NodeType::Ragdoll { settings: Default::default(), view: Default::default(), limits: Default::default() }, p(170.0, 150.0));
    g.add_connection(take, 0, who, 0);
    g.add_connection(who, 0, body, 0);
    g.add_connection(car, 0, body, 1);
    view(g, body, p(170.0, 260.0));
    g.selected_node  = Some(body);
    g.selected_nodes = vec![body];
    format!("Body Collide: the actor walks through the car at frame 1341 and is seated by 1473. Scrub to frame 3000: hips on the seat, feet in the footwell. Bypass the node to see the capture. Display: Hulls shows the shapes that collide. Characterize shows which joint is which part.{}", missing_examples())
}

// ── Martino: composed USD stages and ICE ─────────────────────────────────────

/// One Load USD node on a stage example, viewed and selected.
fn stage(g: &mut NodeGraphState, name: &str, file: &str) -> NodeId {
    crate::graph_io::clear(g);
    let load = g.add_node(name.into(), NodeType::LoadUsd { path: examples::path(file) }, p(180.0, 20.0));
    finish(g, load, load, p(180.0, 130.0));
    load
}

const BOXES: &str = "The Scene Explorer lists the stage: each closed prim is drawn as its bounding box until it is opened, or tick Show all geometry.";

fn street(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    stage(g, "Street", crate::examples_stage::STREET);
    format!("A street from four files: the road and pavements from a sublayer, three tables referenced from table.usda, the shapes on them in the variant \"cafe\", and an override in the root layer that moves the south pavement. {BOXES}{}", missing_examples())
}

fn strands(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    stage(g, "Strands", crate::examples_stage::STRANDS);
    format!("Curves as USD has them: linear, Bezier, Catmull-Rom and a closed B-spline, 600 strands of grass, and points whose width grows along a spiral. {BOXES}{}", missing_examples())
}

fn forest(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let load = g.add_node("Forest".into(), NodeType::LoadUsd { path: examples::path(crate::examples_stage::FOREST) }, p(180.0, 20.0));
    let turn = g.add_node("Turn".into(), transform([0.0; 3], [0.0, 0.35, 0.0], [1.0; 3]), p(180.0, 130.0));
    g.add_connection(load, 0, turn, 0);
    finish(g, turn, load, p(180.0, 240.0));
    format!("4,900 trees and stones from one PointInstancer: three meshes, each drawn once per copy by GPU instancing. The Transform turns the whole forest by moving the placements; the shared meshes are not copied. {BOXES}{}", missing_examples())
}

fn hall(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    stage(g, "Hall", crate::examples_stage::TABLES);
    format!("64 tables, each an instanceable reference to table.usda: the five meshes of the table are read once and placed 64 times. {BOXES}{}", missing_examples())
}

fn lamps(g: &mut NodeGraphState, _: &mut SubnetStore) -> String {
    stage(g, "Lamps", crate::examples_stage::LAMPS);
    format!("Each lamp has render geometry, a proxy of blocks and a guide showing its cone of light. With proxy on, the proxy is drawn in place of the render geometry; switch render, proxy and guide in the Scene Explorer. {BOXES}{}", missing_examples())
}

fn spherify(g: &mut NodeGraphState, store: &mut SubnetStore) -> String {
    crate::graph_io::clear(g);
    let id = store.create_subnet("Spherify".into());
    if let Some(sub) = store.get_mut(id) {
        let (input, out) = (sub.nodes[0].id, sub.nodes[1].id);
        let norm = sub.add_node("Normalize".into(), SubnetNodeType::Normalize, p(220.0, 200.0));
        let grow = sub.add_node("Scale".into(), SubnetNodeType::MultiplyVec3 { scalar: 1.2 }, p(360.0, 200.0));
        sub.add_connection(input, 0, norm, 0);
        sub.add_connection(norm, 0, grow, 0);
        sub.add_connection(grow, 0, out, 0);
    }
    let all = || PolySelection { source: SelSource::All, ..Default::default() };
    let cube = g.add_node("Cube".into(), NodeType::CreateCube { size: 1.0 }, p(180.0, 20.0));
    let dense = g.add_node("Subdivide".into(), edit_poly(vec![PolyOp::new(all(), PolyOpKind::Subdivide { iterations: 3 })], all()), p(180.0, 130.0));
    let tree = g.add_node("Spherify".into(), NodeType::Subnet { id, name: "Spherify".into() }, p(180.0, 240.0));
    g.add_connection(cube, 0, dense, 0);
    g.add_connection(dense, 0, tree, 0);
    finish(g, tree, tree, p(180.0, 350.0));
    "ICE tree: every point of the subdivided cube is set to unit length (Normalize), then scaled by 1.2 (Multiply). Select the Spherify node and press the down arrow to go inside; the up arrow comes back. Change the Multiply value in its properties.".into()
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
    use crate::types::SubnetId;

    #[test]
    fn every_template_is_laid_out_in_clear_rows() {
        use crate::node_graph::ui::{NODE_HEIGHT, NODE_WIDTH};
        for t in TEMPLATES {
            let mut g = NodeGraphState::default();
            t.load(&mut g, &mut SubnetStore::default());
            assert!(g.frame_request, "{}", t.name);
            // No node on top of another.
            for (i, a) in g.nodes.iter().enumerate() {
                for b in &g.nodes[..i] {
                    let apart = (a.position.x - b.position.x).abs() >= NODE_WIDTH + 20.0
                        || (a.position.y - b.position.y).abs() >= NODE_HEIGHT + 20.0;
                    assert!(apart, "{}: {} overlaps {}", t.name, a.name, b.name);
                }
            }
            // Every wire runs downwards, and nothing sits below Output.
            let at = |id| g.nodes.iter().find(|n| n.id == id).unwrap().position;
            for c in &g.connections {
                assert!(at(c.to_node).y > at(c.from_node).y, "{}: a wire runs upwards", t.name);
            }
            let out = at(output(&g)).y;
            assert!(g.nodes.iter().all(|n| n.position.y <= out), "{}", t.name);
        }
    }

    #[test]
    fn a_chain_lays_out_as_a_straight_line() {
        let mut g = NodeGraphState::default();
        let a = g.add_node("A".into(), NodeType::CreateCube { size: 1.0 }, p(500.0, 300.0));
        let b = g.add_node("B".into(), transform([0.0; 3], [0.0; 3], [1.0; 3]), p(-80.0, 10.0));
        g.add_connection(a, 0, b, 0);
        let out = output(&g);
        g.add_connection(b, 0, out, 0);
        g.auto_layout();
        let x: Vec<f32> = [a, b, out].iter().map(|id| g.nodes.iter().find(|n| n.id == *id).unwrap().position.x).collect();
        assert!(x.iter().all(|v| (v - x[0]).abs() < 0.5), "{x:?}");
    }

    #[test]
    fn every_template_builds_something_to_look_at() {
        // The file-based templates need the examples folder of the repository.
        assert!(examples::dir().is_some(), "examples folder not found");

        for t in TEMPLATES {
            assert!(MENU.iter().any(|(by, groups)| *by == t.by && groups.contains(&t.group)), "{}", t.name);
            let mut g = NodeGraphState::default();
            let mut store = SubnetStore::default();
            // Twice: a template has to replace whatever was there.
            (t.build)(&mut g, &mut store);
            let message = (t.build)(&mut g, &mut store);
            assert!(!message.is_empty() && !message.contains("not found"), "{}: {message}", t.name);
            assert_eq!(g.nodes.iter().filter(|n| matches!(n.node_type, NodeType::Output)).count(), 1);

            let eval = |sid: SubnetId, mesh: &crate::core::geo::Geo, template: Option<&crate::core::geo::Geo>| -> crate::core::geo::Geo {
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
    fn uv_templates_give_layouts_in_the_unit_square() {
        for name in ["Unwrap the sea mine", "Edit UV islands"] {
            let t = TEMPLATES.iter().find(|t| t.name == name).unwrap();
            let mut g = NodeGraphState::default();
            (t.build)(&mut g, &mut SubnetStore::default());
            let node = g.selected_node.unwrap();
            let eval = |_: SubnetId, mesh: &crate::core::geo::Geo, _: Option<&crate::core::geo::Geo>| mesh.clone();
            let mesh = g.eval_node(node, &mut std::collections::HashMap::new(), &eval).unwrap().into_mesh();
            assert_eq!(mesh.uvs.len(), mesh.indices.len(), "{name}");
            assert!(mesh.uvs.iter().all(|uv| uv[0].is_finite() && uv[1].is_finite()), "{name}");
            let (_, islands) = crate::core::uv::islands(&mesh);
            if name == "Unwrap the sea mine" {
                assert!(mesh.uvs.iter().all(|uv| uv[0] > -1e-3 && uv[0] < 1.001 && uv[1] > -1e-3 && uv[1] < 1.001));
                assert!(islands > 20 && islands < 2000, "{islands} islands");
                assert!(crate::core::uv::coverage(&mesh) > 0.2, "{}", crate::core::uv::coverage(&mesh));
            } else {
                assert_eq!(islands, 6);
            }
        }
    }

    #[test]
    fn the_ice_template_puts_every_point_on_a_sphere() {
        let t = TEMPLATES.iter().find(|t| t.name == "ICE tree: spherify").unwrap();
        let (mut g, mut store) = (NodeGraphState::default(), SubnetStore::default());
        (t.build)(&mut g, &mut store);
        let eval = |sid: SubnetId, mesh: &crate::core::geo::Geo, template: Option<&crate::core::geo::Geo>| -> crate::core::geo::Geo {
            store.get(sid).map(|sg| sg.evaluate(mesh, template)).unwrap_or_else(|| mesh.clone())
        };
        let mesh = g.eval_node(g.selected_node.unwrap(), &mut std::collections::HashMap::new(), &eval).unwrap().into_mesh();
        assert!(mesh.vertices.len() > 300 && !mesh.indices.is_empty());
        for v in &mesh.vertices {
            let r = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            assert!((r - 1.2).abs() < 1e-4, "{r}");
        }
    }

    /// Added to a scene that has nodes already, the ICE template keeps its
    /// tree: its copy of the tree gives the same sphere.
    #[test]
    fn the_ice_template_added_to_a_scene_still_spherifies() {
        let t = TEMPLATES.iter().find(|t| t.name == "ICE tree: spherify").unwrap();
        let mut store = SubnetStore::default();
        let mut other = NodeGraphState::default();
        t.load(&mut other, &mut store);
        let mut g = NodeGraphState::default();
        g.add_node("Cube".into(), NodeType::CreateCube { size: 1.0 }, p(0.0, 0.0));
        let added = crate::graph_io::add_graph(&mut g, &mut store, &other);
        assert_eq!(added, other.nodes.len() - 1);
        for n in &other.nodes {
            if let NodeType::Subnet { id, .. } = n.node_type { store.subnets.remove(&id); }
        }
        let ice = g.nodes.iter().find(|n| matches!(n.node_type, NodeType::Subnet { .. })).unwrap().id;
        let eval = |sid: SubnetId, mesh: &crate::core::geo::Geo, template: Option<&crate::core::geo::Geo>| -> crate::core::geo::Geo {
            store.get(sid).map(|sg| sg.evaluate(mesh, template)).unwrap_or_else(|| mesh.clone())
        };
        let mesh = g.eval_node(ice, &mut std::collections::HashMap::new(), &eval).unwrap().into_mesh();
        for v in &mesh.vertices {
            let r = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            assert!((r - 1.2).abs() < 1e-4, "{r}");
        }
    }

    #[test]
    fn modelling_templates_give_valid_meshes() {
        for (name, verts_at_least) in [("Edit Poly: tower", 30), ("Edit Poly: panels", 300), ("Edit Poly: goblet", 300), ("Edit Poly: bridge", 20), ("Edit Poly: sea mine", 8000), ("Edit Poly: bolt", 60)] {
            let t = TEMPLATES.iter().find(|t| t.name == name).unwrap();
            let mut g = NodeGraphState::default();
            (t.build)(&mut g, &mut SubnetStore::default());
            let ep = g.nodes.iter().find(|n| matches!(n.node_type, NodeType::EditPoly { .. })).unwrap().id;
            let eval = |_: SubnetId, mesh: &crate::core::geo::Geo, _: Option<&crate::core::geo::Geo>| mesh.clone();
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
