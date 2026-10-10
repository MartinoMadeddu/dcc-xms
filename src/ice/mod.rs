pub mod ui;     // Keep as-is

// NEW - Array-based execution system
pub mod ops;
pub mod ice_nodes;

use std::collections::HashMap;
use bevy::prelude::*;
use bevy_egui::egui;
use crate::types::{
    ConnectionId, MeshData, NodeId, SubnetId,
    SubnetNodeType,
};
use crate::core::geo::Geo;
use ops::Attribute;
use ops::{ExecutionContext, IceNode};  // ✅ Added IceNode trait import!
use ice_nodes::*;

#[derive(Resource, Default)]
pub struct GraphNavigation {
    pub current_subnet: Option<SubnetId>,
}

// ============================================================================
// SUBNET GRAPH STRUCTURES (unchanged)
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
    /// View scale of the tree: 1 is actual size.
    pub zoom:               f32,
}

impl SubnetGraph {
    pub fn new(id: SubnetId, name: String) -> Self {
        let mut g = Self {
            id, name,
            nodes: vec![], connections: vec![],
            next_node_id: 0, next_connection_id: 0,
            selected_node: None, dragging_node: None,
            drag_offset: egui::Vec2::ZERO,
            connecting_from: None, pan_offset: egui::Vec2::ZERO, zoom: 1.0,
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
                (vec![], vec![o("Mesh", "Mesh"), o("Points", "Vec3"), o("Template", "Mesh")]),
            SubnetNodeType::SubOutput =>
                (vec![i("Output", "AnyMesh")], vec![o("Out", "AnyMesh")]),
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
            SubnetNodeType::ScatterPoints { .. } =>
                (vec![i("Geometry", "Mesh")], vec![o("Points", "Points")]),

            SubnetNodeType::GetTemplate =>
                (vec![], vec![o("Template", "Mesh")]),
            SubnetNodeType::CopyToPoints =>
                (vec![i("Points", "Points"), i("Template", "Mesh")], vec![o("Instances", "Mesh")]),
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

    /// Delete a node and the wires to and from it. The tree's input and
    /// output stay: every ICE tree has them. Returns whether it was deleted.
    pub fn remove_node(&mut self, id: NodeId) -> bool {
        let Some(node) = self.nodes.iter().find(|n| n.id == id) else { return false };
        if matches!(node.node_type, SubnetNodeType::SubInput | SubnetNodeType::SubOutput) { return false; }
        let wires: Vec<ConnectionId> = self.connections.iter()
            .filter(|c| c.from_node == id || c.to_node == id)
            .map(|c| c.id)
            .collect();
        for cid in wires { self.remove_connection(cid); }
        self.nodes.retain(|n| n.id != id);
        if self.selected_node == Some(id) { self.selected_node = None; }
        if self.dragging_node == Some(id) { self.dragging_node = None; }
        if self.connecting_from.is_some_and(|(n, _)| n == id) { self.connecting_from = None; }
        true
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

    // ============================================================================
    // NEW EVALUATION - Array-based, graph traversed once
    // ============================================================================

    /// Run the tree on a geometry, in its own space. What the tree does not
    /// write stays shared with `input`. A tree with nothing wired to its
    /// output, or that fails, gives `input` back unchanged.
    pub fn evaluate(&self, input: &Geo, template: Option<&Geo>) -> Geo {
        let Some(sub_out) = self.nodes.iter().find(|n| matches!(n.node_type, SubnetNodeType::SubOutput)) else {
            return input.clone();
        };
        if !self.nodes.iter().any(|n| matches!(n.node_type, SubnetNodeType::SubInput)) {
            return input.clone();
        }
        let Some((src_id, _)) = sub_out.inputs[0].connected_output else { return input.clone() };

        let mut ctx = ExecutionContext::from_geometry(input.clone());
        if let Some(t) = template {
            ctx.add_external_geometry("template", t.clone());
        }
        for node_id in self.get_execution_order(src_id) {
            if let Err(e) = self.execute_node(node_id, &mut ctx) {
                eprintln!("ICE execution error at node {:?}: {}", node_id, e);
                return input.clone();
            }
        }
        ctx.geometry
    }

    /// `evaluate` for code that still holds `MeshData`.
    pub fn evaluate_mesh(&self, input: &MeshData, template: Option<&MeshData>) -> MeshData {
        let template = template.map(Geo::from_mesh);
        self.evaluate(&Geo::from_mesh(input), template.as_ref()).to_mesh()
    }

    /// Get execution order via topological sort (from target backwards)
    fn get_execution_order(&self, target_node: NodeId) -> Vec<NodeId> {
        let mut order = Vec::new();
        let mut visited = std::collections::HashSet::new();
        self.visit_node(target_node, &mut visited, &mut order);
        //order.reverse(); // We built it backwards, reverse for execution order
        order
    }

    fn visit_node(
        &self,
        node_id: NodeId,
        visited: &mut std::collections::HashSet<NodeId>,
        order: &mut Vec<NodeId>,
    ) {
        if visited.contains(&node_id) {
            return;
        }
        visited.insert(node_id);

        // Visit all upstream nodes first
        if let Some(node) = self.nodes.iter().find(|n| n.id == node_id) {
            for input in &node.inputs {
                if let Some((src_id, _)) = input.connected_output {
                    // Don't recurse into SubInput
                    if !self.nodes.iter()
                        .find(|n| n.id == src_id)
                        .map(|n| matches!(n.node_type, SubnetNodeType::SubInput))
                        .unwrap_or(false)
                    {
                        self.visit_node(src_id, visited, order);
                    }
                }
            }
        }

        order.push(node_id);
    }

    /// Execute a single node using the new array-based system
    fn execute_node(&self, node_id: NodeId, ctx: &mut ExecutionContext) -> Result<(), String> {
        let node = self.nodes.iter()
            .find(|n| n.id == node_id)
            .ok_or_else(|| format!("Node {:?} not found", node_id))?;

        match &node.node_type {
            SubnetNodeType::SubInput => {
                // P is already in context, nothing to do
                Ok(())
            }

            SubnetNodeType::SubOutput => {
                // Just pass through - result is already in P
                Ok(())
            }

            SubnetNodeType::AddVec3 => {
                let (a_name, b_name) = self.get_input_attribute_names(node, 0, 1)?;
                let add = AddVec3::new(a_name, b_name, "P"); // Write to P
                add.execute(ctx)
            }

            SubnetNodeType::SubtractVec3 => {
                let (a_name, b_name) = self.get_input_attribute_names(node, 0, 1)?;
                let sub = SubtractVec3::new(a_name, b_name, "P");
                sub.execute(ctx)
            }

            SubnetNodeType::MultiplyVec3 { scalar } => {
                let input_name = self.get_input_attribute_name(node, 0)?;
                let mul = MultiplyVec3::new(input_name, *scalar, "P");
                mul.execute(ctx)
            }

            SubnetNodeType::Normalize => {
                let input_name = self.get_input_attribute_name(node, 0)?;
                let norm = NormalizeVec3::new(input_name, "P");
                norm.execute(ctx)
            }

            SubnetNodeType::CrossProduct => {
                let (a_name, b_name) = self.get_input_attribute_names(node, 0, 1)?;
                let cross = CrossProduct::new(a_name, b_name, "P");
                cross.execute(ctx)
            }

            SubnetNodeType::DotProduct => {
                // TODO: Implement when we have float attributes working
                Err("DotProduct not yet implemented in new system".into())
            }

            SubnetNodeType::LerpVec3 { t } => {
                let (a_name, b_name) = self.get_input_attribute_names(node, 0, 1)?;
                let lerp = LerpVec3::new(a_name, b_name, *t, "P");
                lerp.execute(ctx)
            }

            SubnetNodeType::ConstVec3 { value } => {
                // Create an attribute filled with this constant value
                let point_count = ctx.get_vec3("P")?.len();
                let const_data = vec![*value; point_count];
                ctx.set_vec3(Attribute::new("const_vec3", const_data));
                Ok(())
            }

            SubnetNodeType::ConstFloat { value } => {
                let point_count = ctx.get_vec3("P")?.len();
                let const_data = vec![*value; point_count];
                ctx.set_float(Attribute::new("const_float", const_data));
                Ok(())
            }

            SubnetNodeType::ConstInt { value } => {
                let point_count = ctx.get_vec3("P")?.len();
                let const_data = vec![*value; point_count];
                ctx.set_int(Attribute::new("const_int", const_data));
                Ok(())
            }

            SubnetNodeType::ScatterPoints { count, seed } => {
                let scatter = ice_nodes::ScatterPoints::new(*count, *seed);
                scatter.execute(ctx)
            }
            SubnetNodeType::GetTemplate => {
                let get_template = ice_nodes::GetTemplate::new();
                get_template.execute(ctx)
            }
            
            SubnetNodeType::CopyToPoints => {
                let copy = ice_nodes::CopyToPoints::new();
                copy.execute(ctx)
            }
        }
    }

    /// Helper: Get attribute name from node's input connection
    fn get_input_attribute_name(&self, node: &SubnetNode, input_idx: usize) -> Result<String, String> {
        let input = node.inputs.get(input_idx)
            .ok_or_else(|| format!("Input {} not found", input_idx))?;
        
        match input.connected_output {
            Some((src_id, _)) => {
                // Check if it's SubInput
                if self.nodes.iter()
                    .find(|n| n.id == src_id)
                    .map(|n| matches!(n.node_type, SubnetNodeType::SubInput))
                    .unwrap_or(false)
                {
                    Ok("P".to_string()) // SubInput provides P
                } else {
                    Ok("P".to_string()) // For now, everything reads/writes P
                }
            }
            None => Ok("P".to_string()), // Default to P if not connected
        }
    }

    /// Helper: Get two input attribute names
    fn get_input_attribute_names(&self, node: &SubnetNode, idx_a: usize, idx_b: usize) 
        -> Result<(String, String), String> 
    {
        Ok((
            self.get_input_attribute_name(node, idx_a)?,
            self.get_input_attribute_name(node, idx_b)?,
        ))
    }
}

// ============================================================================
// SUBNET STORE  (unchanged)
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
#[cfg(test)]
mod tree_tests {
    use super::*;
    use std::sync::Arc;
    use crate::core::geo::{Attr, Column, Context, Role, POINTS};

    #[test]
    fn a_tree_runs_on_geo_and_shares_what_it_does_not_write() {
        // In → Multiply by 2 → Out.
        let mut tree = SubnetGraph::new(SubnetId(0), "double".into());
        let (input, output) = (tree.nodes[0].id, tree.nodes[1].id);
        let mul = tree.add_node("Multiply".into(), SubnetNodeType::MultiplyVec3 { scalar: 2.0 }, egui::pos2(0.0, 0.0));
        tree.add_connection(input, 0, mul, 0);
        tree.add_connection(mul, 0, output, 0);

        let mut geo = Geo::from_polygons(vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], vec![3], vec![0, 1, 2]);
        geo.set(Context::Primitive, "heat", Attr::new(Column::Float(Arc::new(vec![0.5])), Role::None));
        let out = tree.evaluate(&geo, None);

        assert_eq!(out.points()[1], [0.0, 2.0, 0.0]);
        assert!(!out.attr(Context::Point, POINTS).unwrap().same(geo.attr(Context::Point, POINTS).unwrap()));
        assert!(out.attr(Context::Primitive, "heat").unwrap().same(geo.attr(Context::Primitive, "heat").unwrap()));
        assert!(out.topology.same(&geo.topology));
        assert!(out.validate().is_ok());
    }

    #[test]
    fn a_tree_with_nothing_wired_gives_its_input_back() {
        let tree = SubnetGraph::new(SubnetId(0), "empty".into());
        let geo = Geo::from_points(vec![[1.0, 2.0, 3.0]]);
        let out = tree.evaluate(&geo, None);
        assert!(out.attr(Context::Point, POINTS).unwrap().same(geo.attr(Context::Point, POINTS).unwrap()));
    }
}
