pub mod nodes;
pub mod ui;

use std::collections::HashMap;
use bevy::prelude::*;
use bevy_egui::egui;
use crate::types::{
    ConnectionId, MeshData, NodeId, SubnetId,
    SubnetNodeType, SubnetValue,
};
use nodes::evaluate_subnet_node;

#[derive(Resource, Default)]
pub struct GraphNavigation {
    pub current_subnet: Option<SubnetId>,
}

// ============================================================================
// SUBNET GRAPH STRUCTURES
// ============================================================================

#[derive(Clone)]
pub struct SubnetInputSocket {
    pub name:             String,
    pub value_hint:       &'static str,
    pub connected_output: Option<(NodeId, usize)>,
}

#[derive(Clone)]
pub struct SubnetOutputSocket {
    pub name:       String,
    pub value_hint: &'static str,
}

#[derive(Clone)]
pub struct SubnetNode {
    pub id:        NodeId,
    pub name:      String,
    pub node_type: SubnetNodeType,
    pub position:  egui::Pos2,
    pub inputs:    Vec<SubnetInputSocket>,
    pub outputs:   Vec<SubnetOutputSocket>,
}

#[derive(Clone)]
pub struct SubnetConnection {
    pub id:          ConnectionId,
    pub from_node:   NodeId,
    pub from_output: usize,
    pub to_node:     NodeId,
    pub to_input:    usize,
}

// ============================================================================
// SUBNET GRAPH
// ============================================================================

#[derive(Clone)]
pub struct SubnetGraph {
    pub id:                 SubnetId,
    pub name:               String,
    pub nodes:              Vec<SubnetNode>,
    pub connections:        Vec<SubnetConnection>,
    pub next_node_id:       usize,
    pub next_connection_id: usize,
    pub selected_node:      Option<NodeId>,
    pub dragging_node:      Option<NodeId>,
    pub drag_offset:        egui::Vec2,
    pub connecting_from:    Option<(NodeId, usize)>,
    pub pan_offset:         egui::Vec2,
}

impl SubnetGraph {
    pub fn new(id: SubnetId, name: String) -> Self {
        let mut g = Self {
            id, name,
            nodes: vec![], connections: vec![],
            next_node_id: 0, next_connection_id: 0,
            selected_node: None, dragging_node: None,
            drag_offset: egui::Vec2::ZERO,
            connecting_from: None, pan_offset: egui::Vec2::ZERO,
        };
        g.add_node("SubInput".into(),  SubnetNodeType::SubInput,  egui::pos2(60.0,  200.0));
        g.add_node("SubOutput".into(), SubnetNodeType::SubOutput, egui::pos2(500.0, 200.0));
        g
    }

    pub fn add_node(&mut self, name: String, t: SubnetNodeType, pos: egui::Pos2) -> NodeId {
        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;
        let (inputs, outputs) = Self::create_sockets(&t);
        self.nodes.push(SubnetNode { id, name, node_type: t, position: pos, inputs, outputs });
        id
    }

    pub fn create_sockets(t: &SubnetNodeType)
        -> (Vec<SubnetInputSocket>, Vec<SubnetOutputSocket>)
    {
        let i = |n: &str, h: &'static str| SubnetInputSocket {
            name: n.into(), value_hint: h, connected_output: None,
        };
        let o = |n: &str, h: &'static str| SubnetOutputSocket {
            name: n.into(), value_hint: h,
        };
        match t {
            SubnetNodeType::SubInput =>
                (vec![], vec![o("Mesh", "Mesh"), o("Points", "Vec3")]),
            SubnetNodeType::SubOutput =>
                (vec![i("Points", "Vec3"), i("Mesh", "Mesh")], vec![o("Out", "Mesh")]),
            SubnetNodeType::AddVec3 | SubnetNodeType::SubtractVec3 | SubnetNodeType::CrossProduct =>
                (vec![i("A", "Vec3"), i("B", "Vec3")], vec![o("Result", "Vec3")]),
            SubnetNodeType::MultiplyVec3 { .. } =>
                (vec![i("Vec", "Vec3")], vec![o("Result", "Vec3")]),
            SubnetNodeType::Normalize =>
                (vec![i("Vec", "Vec3")], vec![o("Result", "Vec3")]),
            SubnetNodeType::DotProduct =>
                (vec![i("A", "Vec3"), i("B", "Vec3")], vec![o("Value", "Float")]),
            SubnetNodeType::LerpVec3 { .. } =>
                (vec![i("A", "Vec3"), i("B", "Vec3")], vec![o("Result", "Vec3")]),
            SubnetNodeType::ConstVec3  { .. } => (vec![], vec![o("Value", "Vec3")]),
            SubnetNodeType::ConstFloat { .. } => (vec![], vec![o("Value", "Float")]),
            SubnetNodeType::ConstInt   { .. } => (vec![], vec![o("Value", "Int")]),
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
        self.connections.push(SubnetConnection {
            id,
            from_node: from, from_output: from_out,
            to_node:   to,   to_input:    to_in,
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

    pub fn evaluate(&self, input_mesh: &MeshData) -> MeshData {
        let mut cache: HashMap<NodeId, Vec<SubnetValue>> = HashMap::new();

        if let Some(sub_in) = self.nodes.iter().find(|n|
            matches!(n.node_type, SubnetNodeType::SubInput))
        {
            let points: Vec<SubnetValue> = input_mesh.vertices.iter()
                .map(|v| SubnetValue::Vec3(Vec3::from_array(*v)))
                .collect();
            cache.insert(sub_in.id, vec![
                SubnetValue::Mesh(input_mesh.clone()),
                if points.is_empty() { SubnetValue::Vec3(Vec3::ZERO) } else { points[0].clone() },
            ]);
        }

        let sub_out = match self.nodes.iter().find(|n|
            matches!(n.node_type, SubnetNodeType::SubOutput))
        {
            Some(n) => n,
            None    => return input_mesh.clone(),
        };

        if let Some(inp) = sub_out.inputs.first() {
            if let Some((src_id, src_out)) = inp.connected_output {
                let out_verts: Vec<[f32; 3]> = input_mesh.vertices.iter()
                    .map(|v| {
                        let point_val = SubnetValue::Vec3(Vec3::from_array(*v));
                        let result = self.eval_node_for_point(src_id, src_out, &point_val, input_mesh);
                        match result {
                            SubnetValue::Vec3(v3) => v3.to_array(),
                            _ => *v,
                        }
                    })
                    .collect();
                return MeshData {
                    vertices: out_verts,
                    indices:  input_mesh.indices.clone(),
                    points:   input_mesh.points.clone(),
                        ..Default::default()
                };
            }
        }

        input_mesh.clone()
    }

    fn eval_node_for_point(
        &self,
        node_id: NodeId,
        out_idx: usize,
        point:   &SubnetValue,
        mesh:    &MeshData,
    ) -> SubnetValue {
        let node = match self.nodes.iter().find(|n| n.id == node_id) {
            Some(n) => n,
            None    => return point.clone(),
        };

        match &node.node_type {
            SubnetNodeType::ConstVec3  { value } => return SubnetValue::Vec3(*value),
            SubnetNodeType::ConstFloat { value } => return SubnetValue::Float(*value),
            SubnetNodeType::ConstInt   { value } => return SubnetValue::Int(*value),
            _ => {}
        }

        let inputs: Vec<SubnetValue> = node.inputs.iter().map(|sock| {
            match sock.connected_output {
                Some((src_id, src_out)) => {
                    if self.nodes.iter().find(|n| n.id == src_id)
                        .map(|n| matches!(n.node_type, SubnetNodeType::SubInput))
                        .unwrap_or(false)
                    {
                        if src_out == 0 { SubnetValue::Mesh(mesh.clone()) }
                        else            { point.clone() }
                    } else {
                        self.eval_node_for_point(src_id, src_out, point, mesh)
                    }
                }
                None => point.clone(),
            }
        }).collect();

        let results = evaluate_subnet_node(&node.node_type, &inputs);
        results.into_iter().nth(out_idx).unwrap_or_else(|| point.clone())
    }
}

// ============================================================================
// SUBNET STORE  (Bevy Resource)
// ============================================================================

#[derive(Resource, Default)]
pub struct SubnetStore {
    pub subnets: HashMap<SubnetId, SubnetGraph>,
    pub next_id: usize,
}

impl SubnetStore {
    pub fn create_subnet(&mut self, name: String) -> SubnetId {
        let id = SubnetId(self.next_id);
        self.next_id += 1;
        self.subnets.insert(id, SubnetGraph::new(id, name));
        id
    }
    pub fn get(&self,         id: SubnetId) -> Option<&SubnetGraph>     { self.subnets.get(&id)     }
    pub fn get_mut(&mut self, id: SubnetId) -> Option<&mut SubnetGraph> { self.subnets.get_mut(&id) }
}