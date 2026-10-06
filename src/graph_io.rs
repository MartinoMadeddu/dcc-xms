//! Saving and loading the node graph as JSON, and ready-made graphs.
//!
//! The contents of ICE subnets are not saved: a loaded Subnet node starts
//! with a fresh, empty subnet.

use std::path::Path;

use bevy_egui::egui;
use serde::{Deserialize, Serialize};

use crate::node_graph::NodeGraphState;
use crate::types::{NodeId, NodeType, SplitPick, SubnetId, DEFAULT_WRITE_PATH};

#[derive(Serialize, Deserialize)]
struct SavedNode {
    id:        usize,
    name:      String,
    position:  [f32; 2],
    node_type: NodeType,
}

#[derive(Serialize, Deserialize)]
struct SavedGraph {
    version:     u32,
    nodes:       Vec<SavedNode>,
    /// from node, from output, to node, to input
    connections: Vec<[usize; 4]>,
    view_flag:   Option<usize>,
}

pub fn to_json(graph: &NodeGraphState) -> String {
    let saved = SavedGraph {
        version: 1,
        nodes: graph.nodes.iter().map(|n| SavedNode {
            id:        n.id.0,
            name:      n.name.clone(),
            position:  [n.position.x, n.position.y],
            node_type: n.node_type.clone(),
        }).collect(),
        connections: graph.connections.iter()
            .map(|c| [c.from_node.0, c.from_output, c.to_node.0, c.to_input])
            .collect(),
        view_flag: graph.view_flag.map(|n| n.0),
    };
    serde_json::to_string_pretty(&saved).unwrap_or_default()
}

/// Replace the contents of `graph` with a saved graph. View settings (pan,
/// zoom) are kept.
pub fn from_json(graph: &mut NodeGraphState, json: &str) -> Result<(), String> {
    let saved: SavedGraph = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if !saved.nodes.iter().any(|n| matches!(n.node_type, NodeType::Output)) {
        return Err("graph has no Output node".into());
    }
    clear(graph);
    graph.nodes.clear();
    for n in saved.nodes {
        let mut node_type = n.node_type;
        if let NodeType::Subnet { id, .. } = &mut node_type { *id = SubnetId(usize::MAX); }
        let id = graph.add_node(n.name, node_type, egui::pos2(n.position[0], n.position[1]));
        // Keep the saved ids so connections can be restored as written.
        if let Some(node) = graph.nodes.iter_mut().find(|x| x.id == id) { node.id = NodeId(n.id); }
        graph.next_node_id = graph.next_node_id.max(n.id + 1);
    }
    for [from, out, to, input] in saved.connections {
        let ok = graph.nodes.iter().any(|n| n.id.0 == from && out < n.outputs.len())
              && graph.nodes.iter().any(|n| n.id.0 == to && input < n.inputs.len());
        if ok { graph.add_connection(NodeId(from), out, NodeId(to), input); }
    }
    graph.view_flag = saved.view_flag.map(NodeId).filter(|v| graph.nodes.iter().any(|n| n.id == *v));
    Ok(())
}

pub fn save(graph: &NodeGraphState, path: &Path) -> Result<(), String> {
    std::fs::write(path, to_json(graph)).map_err(|e| e.to_string())
}

pub fn load(graph: &mut NodeGraphState, path: &Path) -> Result<(), String> {
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    from_json(graph, &json)
}

/// Remove everything except the Output node.
pub(crate) fn clear(graph: &mut NodeGraphState) {
    graph.connections.clear();
    graph.nodes.retain(|n| matches!(n.node_type, NodeType::Output));
    for n in &mut graph.nodes {
        for i in &mut n.inputs { i.connected_output = None; }
    }
    graph.selected_node = None;
    graph.selected_nodes.clear();
    graph.selected_connection = None;
    graph.view_flag = None;
    graph.dragging_node = None;
    graph.connecting_from = None;
}

/// Graph that splits a two-character mocap take for Unreal: per character,
/// one FBX with the animation and one skinned T-pose FBX.
pub fn mocap_split_template(graph: &mut NodeGraphState) {
    clear(graph);
    let p = |x: f32, y: f32| egui::pos2(x, y);

    let load = graph.add_node("Takes".into(),
        NodeType::LoadFbxDir { dir: String::new(), index: 0, take: 0 }, p(330.0, 20.0));
    let split = graph.add_node("Split".into(),
        NodeType::SplitSkeleton { picks: vec![SplitPick::Character(0), SplitPick::Character(1)] }, p(330.0, 120.0));
    graph.add_connection(load, 0, split, 0);

    let tpose_path = DEFAULT_WRITE_PATH.replace(".fbx", "_tpose.fbx");
    for c in 0..2usize {
        let x = 20.0 + c as f32 * 420.0;
        let n = c + 1;
        let anim = graph.add_node(format!("WriteAnim{n}"),
            NodeType::WriteFbx { path: DEFAULT_WRITE_PATH.into() }, p(x, 240.0));
        let tpose = graph.add_node(format!("TPose{n}"),
            NodeType::AutoTPose { set_hip_height: false, hip_height: 90.0 }, p(x + 200.0, 240.0));
        let fix = graph.add_node(format!("FixPose{n}"),
            NodeType::FixPose { edits: vec![] }, p(x + 200.0, 340.0));
        let skin = graph.add_node(format!("ProxySkin{n}"),
            NodeType::ProxySkin { thickness: 1.0 }, p(x + 200.0, 440.0));
        let write = graph.add_node(format!("WriteTPose{n}"),
            NodeType::WriteFbx { path: tpose_path.clone() }, p(x + 200.0, 540.0));
        graph.add_connection(split, c, anim, 0);
        graph.add_connection(split, c, tpose, 0);
        graph.add_connection(tpose, 0, fix, 0);
        graph.add_connection(fix, 0, skin, 0);
        graph.add_connection(skin, 0, write, 0);
    }

    if let Some(out) = graph.nodes.iter_mut().find(|n| matches!(n.node_type, NodeType::Output)) {
        out.position = p(330.0, 660.0);
    }
    graph.view_flag     = Some(split);
    graph.selected_node = Some(load);
    graph.selected_nodes = vec![load];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_survives_save_and_load() {
        let mut g = NodeGraphState::default();
        mocap_split_template(&mut g);
        let cube = g.add_node("Cube".into(), NodeType::CreateCube { size: 2.0 }, egui::pos2(1.0, 2.0));
        let xf = g.add_node("Xf".into(), NodeType::Transform {
            translation: bevy::math::Vec3::new(1.0, 2.0, 3.0),
            rotation: bevy::math::Vec3::ZERO, scale: bevy::math::Vec3::ONE,
        }, egui::pos2(5.0, 6.0));
        g.add_connection(cube, 0, xf, 0);
        // An Edit Poly node with an operation, a rule-based and a picked selection.
        use crate::core::poly::{ExtrudeMode, PolyOp, PolyOpKind, PolySelection, SelSource, SubLevel};
        let ep = g.add_node("EditPoly".into(), NodeType::EditPoly {
            ops: vec![
                PolyOp {
                    enabled: true,
                    selection: PolySelection { source: SelSource::ByNormal { dir: [0.0, 1.0, 0.0], angle: 5.0 }, grow: 1, ..Default::default() },
                    kind: PolyOpKind::Bevel { height: 0.5, outline: -0.1, mode: ExtrudeMode::LocalNormal },
                    collapsed: true,
                },
                PolyOp::new(
                    PolySelection { level: SubLevel::Border, ..Default::default() },
                    PolyOpKind::Transform { translate: [0.5, 0.0, 0.25], rotate: [0.0, 0.0, 0.0, 1.0], scale: [1.0, 2.0, 1.0] }),
                PolyOp::new(Default::default(), PolyOpKind::MakePlanar { axis: None }),
            ],
            pending: PolySelection { level: SubLevel::Edge, edges: vec![[0, 1], [2, 3]], ..Default::default() },
            edit: Some(0),
            auto_collapse: true,
        }, egui::pos2(9.0, 9.0));
        g.add_connection(xf, 0, ep, 0);
        let json = to_json(&g);

        let mut h = NodeGraphState::default();
        from_json(&mut h, &json).unwrap();
        assert_eq!(h.nodes.len(), g.nodes.len());
        assert_eq!(h.connections.len(), g.connections.len());
        assert_eq!(h.view_flag, g.view_flag);
        assert_eq!(to_json(&h), json);
        // Split keeps both outputs and their wires.
        let split = h.nodes.iter().find(|n| matches!(n.node_type, NodeType::SplitSkeleton { .. })).unwrap();
        assert_eq!(split.outputs.len(), 2);
        assert_eq!(h.connections.iter().filter(|c| c.from_node == split.id && c.from_output == 1).count(), 2);
        // New nodes do not collide with restored ids.
        let fresh = h.add_node("New".into(), NodeType::Merge, egui::pos2(0.0, 0.0));
        assert_eq!(h.nodes.iter().filter(|n| n.id == fresh).count(), 1);
    }

    #[test]
    fn bad_json_leaves_graph_alone() {
        let mut g = NodeGraphState::default();
        mocap_split_template(&mut g);
        let before = to_json(&g);
        assert!(from_json(&mut g, "{ not json").is_err());
        assert!(from_json(&mut g, r#"{"version":1,"nodes":[],"connections":[],"view_flag":null}"#).is_err());
        assert_eq!(to_json(&g), before);
    }
}
