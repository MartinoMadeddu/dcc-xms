pub mod nodes;
pub mod ui;

use std::collections::HashMap;
use bevy::prelude::*;
use bevy_egui::egui;
use crate::types::{ConnectionId, EvalResult, MeshData, NodeId, NodeType, SubnetId};
use nodes::evaluate_node_type;

// ============================================================================
// GRAPH DATA STRUCTURES
// ============================================================================

#[derive(Clone)]
pub struct InputSocket {
    pub name:             String,
    pub connected_output: Option<(NodeId, usize)>,
}

#[derive(Clone)]
pub struct OutputSocket {
    pub name: String,
}

#[derive(Clone)]
pub struct GraphNode {
    pub id:        NodeId,
    pub name:      String,
    pub node_type: NodeType,
    pub position:  egui::Pos2,
    pub inputs:    Vec<InputSocket>,
    pub outputs:   Vec<OutputSocket>,
}

#[derive(Clone)]
pub struct Connection {
    pub id:          ConnectionId,
    pub from_node:   NodeId,
    pub from_output: usize,
    pub to_node:     NodeId,
    pub to_input:    usize,
}

// ============================================================================
// NODE GRAPH STATE  (Bevy Resource)
// ============================================================================

#[derive(Resource)]
pub struct NodeGraphState {
    pub nodes:               Vec<GraphNode>,
    pub connections:         Vec<Connection>,
    pub next_node_id:        usize,
    pub next_connection_id:  usize,
    pub selected_node:       Option<NodeId>,
    pub dragging_node:       Option<NodeId>,
    pub drag_offset:         egui::Vec2,
    pub connecting_from:     Option<(NodeId, usize)>,
    pub pan_offset:          egui::Vec2,
    pub renaming_node:       Option<NodeId>,
    pub rename_buffer:       String,
    pub tab_menu_screen_pos: Option<egui::Pos2>,
    pub tab_menu_canvas_pos: Option<egui::Pos2>,
    pub zoom:                f32,
    pub selected_nodes:     Vec<NodeId>,
    pub marquee_start:      Option<egui::Pos2>,
    pub selected_connection: Option<ConnectionId>,
}

impl Default for NodeGraphState {
    fn default() -> Self {
        let mut s = Self {
            nodes: vec![], connections: vec![],
            next_node_id: 0, next_connection_id: 0,
            selected_node: None, dragging_node: None,
            drag_offset: egui::Vec2::ZERO,
            connecting_from: None, pan_offset: egui::Vec2::ZERO,
            renaming_node: None, rename_buffer: String::new(),
            tab_menu_screen_pos: None, tab_menu_canvas_pos: None,
            zoom: 1.0,
            selected_nodes: vec![],
            marquee_start:  None,
            selected_connection: None,
        };
        s.add_node("Output".into(), NodeType::Output, egui::pos2(200.0, 400.0));
        s
    }
}

impl NodeGraphState {
    pub fn add_node(&mut self, name: String, node_type: NodeType, pos: egui::Pos2) -> NodeId {
        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;
        let (inputs, outputs) = Self::create_sockets(&node_type);
        self.nodes.push(GraphNode { id, name, node_type, position: pos, inputs, outputs });
        id
    }

    pub fn create_sockets(t: &NodeType) -> (Vec<InputSocket>, Vec<OutputSocket>) {
        let i = |n: &str| InputSocket  { name: n.into(), connected_output: None };
        let o = |n: &str| OutputSocket { name: n.into() };
        match t {
            NodeType::CreateCube { .. }
            | NodeType::CreateSphere { .. }
            | NodeType::CreateGrid { .. }
            | NodeType::LoadUsd { .. }      => (vec![], vec![o("Mesh")]),
            NodeType::Transform { .. }      => (vec![i("Input")], vec![o("Output")]),
            NodeType::Merge                 => (vec![i("A"), i("B")], vec![o("Result")]),
            NodeType::ScatterPoints { .. }  => (vec![i("Surface")], vec![o("Points")]),
            NodeType::CopyToPoints          => (vec![i("Template"), i("Points")], vec![o("Geo")]),
            NodeType::Output                => (vec![i("Scene")], vec![]),
            NodeType::Subnet { .. }         => (vec![i("In")], vec![o("Out")]),
        }
    }

    pub fn add_connection(&mut self, from: NodeId, from_out: usize, to: NodeId, to_in: usize) {
        self.connections.retain(|c| !(c.to_node == to && c.to_input == to_in));
        let id = ConnectionId(self.next_connection_id);
        self.next_connection_id += 1;
        if let Some(n) = self.nodes.iter_mut().find(|n| n.id == to) {
            if let Some(inp) = n.inputs.get_mut(to_in) {
                inp.connected_output = Some((from, from_out));
            }
        }
        self.connections.push(Connection {
            id,
            from_node: from, from_output: from_out,
            to_node: to,     to_input: to_in,
        });
    }

    pub fn remove_connection(&mut self, cid: ConnectionId) {
        if let Some(c) = self.connections.iter().find(|c| c.id == cid) {
            let (tn, ti) = (c.to_node, c.to_input);
            if let Some(n) = self.nodes.iter_mut().find(|n| n.id == tn) {
                if let Some(inp) = n.inputs.get_mut(ti) { inp.connected_output = None; }
            }
        }
        self.connections.retain(|c| c.id != cid);
    }
    pub fn delete_selected(&mut self) {
        // Delete selected connection first (if any), nodes take priority if both set
        if !self.selected_nodes.is_empty() {
            let to_delete: Vec<NodeId> = self.selected_nodes.drain(..).collect();
            for nid in &to_delete {
                // Skip the Output node — it can't be deleted
                if let Some(n) = self.nodes.iter().find(|n| n.id == *nid) {
                    if matches!(n.node_type, NodeType::Output) { continue; }
                }
                // Remove all connections involving this node
                let conns_to_remove: Vec<ConnectionId> = self.connections.iter()
                    .filter(|c| c.from_node == *nid || c.to_node == *nid)
                    .map(|c| c.id)
                    .collect();
                for cid in conns_to_remove { self.remove_connection(cid); }
                self.nodes.retain(|n| n.id != *nid);
            }
            if self.selected_node.map(|id| to_delete.contains(&id)).unwrap_or(false) {
                self.selected_node = None;
            }
            self.selected_connection = None;
        } else if let Some(cid) = self.selected_connection.take() {
            self.remove_connection(cid);
        }
    }

    // ── Viewport evaluation ───────────────────────────────────────────────────
    // Evaluates the full operator chain from Output downward.
    // Transforms, subnets, merges etc. all apply correctly because we follow
    // the chain top-down and pass each node's result to the next.
    // Returns the single final MeshData connected to Output (what the viewport shows).
    pub fn evaluate_for_viewport(
        &self,
        eval_subnet: &impl Fn(SubnetId, &MeshData) -> MeshData,
    ) -> Option<MeshData> {
        let root = self.nodes.iter().find(|n| matches!(n.node_type, NodeType::Output))?;
        let (src, _) = root.inputs.first()?.connected_output?;
        let mut cache = HashMap::new();
        self.eval_node(src, &mut cache, eval_subnet)
            .map(|r| r.into_mesh())
    }

    // Recursive bottom-up evaluator — follows the full chain.
    pub fn eval_node(
        &self,
        id:          NodeId,
        cache:       &mut HashMap<NodeId, Option<EvalResult>>,
        eval_subnet: &impl Fn(SubnetId, &MeshData) -> MeshData,
    ) -> Option<EvalResult> {
        if let Some(cached) = cache.get(&id) { return cached.clone(); }

        let node = self.nodes.iter().find(|n| n.id == id)?;

        let inputs: Vec<EvalResult> = node.inputs.iter()
            .filter_map(|s| s.connected_output
                .and_then(|(src, _)| self.eval_node(src, cache, eval_subnet)))
            .collect();

        let result = evaluate_node_type(&node.node_type, &inputs, eval_subnet);
        cache.insert(id, result.clone());
        result
    }

    // ── Scene explorer evaluation ─────────────────────────────────────────────
    // Separate from viewport eval. Walks the graph to collect generator nodes
    // (Cube, Sphere, LoadUsd…) with their names and raw EvalResults for the
    // scene hierarchy UI. Operators are transparent here — only leaf generators
    // appear as scene objects.
    pub fn evaluate_for_scene(
        &self,
        eval_subnet: &impl Fn(SubnetId, &MeshData) -> MeshData,
    ) -> Vec<(NodeId, String, EvalResult)> {
        let mut cache  = HashMap::new();
        let mut out    = vec![];
        let mut visited = std::collections::HashSet::new();

        // Only walk nodes reachable from Output
        if let Some(root) = self.nodes.iter().find(|n| matches!(n.node_type, NodeType::Output)) {
            if let Some(inp) = root.inputs.first() {
                if let Some((src, _)) = inp.connected_output {
                    self.walk_for_scene(src, &mut cache, eval_subnet, &mut out, &mut visited);
                }
            }
        }
        out
    }

    fn walk_for_scene(
        &self,
        id:          NodeId,
        cache:       &mut HashMap<NodeId, Option<EvalResult>>,
        eval_subnet: &impl Fn(SubnetId, &MeshData) -> MeshData,
        out:         &mut Vec<(NodeId, String, EvalResult)>,
        visited:     &mut std::collections::HashSet<NodeId>,
    ) {
        if !visited.insert(id) { return; }

        let node = match self.nodes.iter().find(|n| n.id == id) {
            Some(n) => n,
            None    => return,
        };

        // Recurse into upstream nodes first
        for inp in &node.inputs {
            if let Some((src, _)) = inp.connected_output {
                self.walk_for_scene(src, cache, eval_subnet, out, visited);
            }
        }

        // Evaluate this node (uses cache so work isn't duplicated)
        if !cache.contains_key(&id) {
            let inputs: Vec<EvalResult> = node.inputs.iter()
                .filter_map(|s| s.connected_output
                    .and_then(|(src, _)| cache.get(&src).and_then(|r| r.clone())))
                .collect();
            let result = evaluate_node_type(&node.node_type, &inputs, eval_subnet);
            cache.insert(id, result);
        }

        // Only generator nodes appear in the scene explorer
        if let Some(r) = cache.get(&id).and_then(|r| r.clone()) {
            match &node.node_type {
                NodeType::CreateCube { .. }
                | NodeType::CreateSphere { .. }
                | NodeType::CreateGrid { .. }
                | NodeType::LoadUsd { .. } => {
                    out.push((node.id, node.name.clone(), r));
                }
                _ => {}
            }
        }
    }
}