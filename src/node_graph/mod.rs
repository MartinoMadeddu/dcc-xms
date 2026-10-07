pub mod nodes;
pub mod ui;

use std::collections::HashMap;
use bevy::prelude::*;
use bevy_egui::egui;
use crate::types::{ConnectionId, EvalResult, MeshData, NodeId, NodeType, SubnetId};
use nodes::evaluate_node_type;

/// Results of one evaluation pass, per node output socket.
pub type EvalCache = HashMap<(NodeId, usize), Option<EvalResult>>;

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

#[derive(Resource, Clone)]
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
    pub selected_nodes:      Vec<NodeId>,
    pub marquee_start:       Option<egui::Pos2>,
    pub selected_connection: Option<ConnectionId>,
    pub graph_version:       u64,
    pub view_flag:           Option<NodeId>,  // Which node viewport displays
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
            graph_version: 0,
            view_flag: None,
        };
        s.add_node("Output".into(), NodeType::Output, egui::pos2(200.0, 400.0));
        s
    }
}

/// Counts changes to what the graph evaluates to or shows. Systems that
/// cook the graph run when this changes, not on every frame.
#[derive(bevy::prelude::Resource, Default)]
pub struct GraphRevision(pub u64);

impl NodeGraphState {
    /// Hash of everything evaluation and display depend on: nodes with their
    /// parameters, wires, the view flag and the selection. Node positions,
    /// panning and drags in progress are left out.
    pub fn content_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        struct W(std::collections::hash_map::DefaultHasher);
        impl std::io::Write for W {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> { self.0.write(buf); Ok(buf.len()) }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let mut w = W(Default::default());
        for n in &self.nodes {
            n.id.0.hash(&mut w.0);
            n.name.hash(&mut w.0);
            let _ = serde_json::to_writer(&mut w, &n.node_type);
            for i in &n.inputs { i.connected_output.map(|(id, out)| (id.0, out)).hash(&mut w.0); }
            n.outputs.len().hash(&mut w.0);
        }
        for c in &self.connections { (c.from_node.0, c.from_output, c.to_node.0, c.to_input).hash(&mut w.0); }
        self.view_flag.map(|n| n.0).hash(&mut w.0);
        self.selected_node.map(|n| n.0).hash(&mut w.0);
        for n in &self.selected_nodes { n.0.hash(&mut w.0); }
        w.0.finish()
    }

    // ✅ Increment version whenever graph changes
    fn mark_dirty(&mut self) {
        self.graph_version = self.graph_version.wrapping_add(1);
    }

    pub fn add_node(&mut self, name: String, node_type: NodeType, pos: egui::Pos2) -> NodeId {
        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;
        let (inputs, outputs) = Self::create_sockets(&node_type);
        self.nodes.push(GraphNode { id, name, node_type, position: pos, inputs, outputs });
        self.mark_dirty();
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
            NodeType::Subnet { .. }         => (vec![i("Geometry"), i("Template")], vec![o("Out")]),
            NodeType::LoadFbx { .. }
            | NodeType::TestClip { .. }     => (vec![], vec![o("Clip")]),
            NodeType::RenameJoints { .. }
            | NodeType::TrimClip { .. }
            | NodeType::Retime { .. }
            | NodeType::SetTimecode { .. }
            | NodeType::AutoTPose { .. }
            | NodeType::FixPose { .. }
            | NodeType::ProxySkin { .. }
            | NodeType::WriteFbx { .. }
            | NodeType::MirrorClip
            | NodeType::SmoothClip { .. }
            | NodeType::InPlace { .. }
            | NodeType::TransformClip { .. }
            | NodeType::LoopClip { .. }
            | NodeType::TimeWarp { .. }
            | NodeType::PruneJoints { .. }
            | NodeType::FloorClip { .. }    => (vec![i("Clip")], vec![o("Clip")]),
            NodeType::BlendClips { .. }     => (vec![i("First"), i("Next")], vec![o("Clip")]),
            NodeType::Retarget              => (vec![i("Motion"), i("Skeleton")], vec![o("Clip")]),
            NodeType::UvUnwrap { .. }
            | NodeType::UvTransform { .. }
            | NodeType::UvEdit { .. }       => (vec![i("Mesh")], vec![o("Mesh")]),
            NodeType::LoadFbxDir { .. }     => (vec![], vec![o("Clip")]),
            NodeType::EditPoly { .. }       => (vec![i("Mesh")], vec![o("Mesh")]),
            NodeType::SplitSkeleton { picks } => (
                vec![i("Clip")],
                (0..picks.len()).map(|n| OutputSocket { name: format!("Char {}", n + 1) }).collect(),
            ),
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
        self.mark_dirty();
    }

    pub fn remove_connection(&mut self, cid: ConnectionId) {
        if let Some(c) = self.connections.iter().find(|c| c.id == cid) {
            let (tn, ti) = (c.to_node, c.to_input);
            if let Some(n) = self.nodes.iter_mut().find(|n| n.id == tn) {
                if let Some(inp) = n.inputs.get_mut(ti) { inp.connected_output = None; }
            }
        }
        self.connections.retain(|c| c.id != cid);
        self.mark_dirty();
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
                // 👁️ NEW - Clear view flag if deleting the viewed node
                if self.view_flag == Some(*nid) {
                    self.view_flag = None;
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
        self.mark_dirty();
    }

    // ============================================================================
    // 👁️ NEW - VIEW FLAG SYSTEM
    // ============================================================================

    /// Toggle the view flag on a node
    /// If the node already has the view flag, remove it (revert to Output)
    /// Otherwise, set the view flag to this node
    pub fn toggle_view_flag(&mut self, node_id: NodeId) {
        if self.view_flag == Some(node_id) {
            // Remove view flag (will show Output)
            self.view_flag = None;
        } else {
            // Set view flag to this node
            self.view_flag = Some(node_id);
        }
    }

    /// Check if a specific node has the view flag
    pub fn has_view_flag(&self, node_id: NodeId) -> bool {
        self.view_flag == Some(node_id)
    }

    /// Get the node ID that should be displayed in the viewport
    /// Returns the view flag node if set, otherwise returns Output node
    pub fn get_viewport_node(&self) -> Option<NodeId> {
        if let Some(flagged_node) = self.view_flag {
            // View flag is set, use that node
            return Some(flagged_node);
        }
        
        // No view flag, fallback to Output node
        self.nodes.iter()
            .find(|n| matches!(n.node_type, NodeType::Output))
            .map(|n| n.id)
    }

    /// Clear the view flag (revert to showing Output)
    pub fn clear_view_flag(&mut self) {
        self.view_flag = None;
    }

    // ============================================================================
    // END VIEW FLAG SYSTEM
    // ============================================================================

    // ── Viewport evaluation ───────────────────────────────────────────────────
    // 👁️ MODIFIED - Now respects view flag
    // Evaluates from the view flag node if set, otherwise from Output.
    // This allows viewing intermediate results in the node chain.
    pub fn evaluate_for_viewport(
        &self,
        eval_subnet: &impl Fn(SubnetId, &MeshData, Option<&MeshData>) -> MeshData,
    ) -> Option<MeshData> {
        // 👁️ Use view flag if set, otherwise use Output
        let display_node_id = self.get_viewport_node()?;
        
        // If viewing a node with no output (like Output itself), walk upstream
        let node = self.nodes.iter().find(|n| n.id == display_node_id)?;
        
        // If this is the Output node, get its input
        if matches!(node.node_type, NodeType::Output) {
            let (src, out) = node.inputs.first()?.connected_output?;
            let mut cache = HashMap::new();
            return self.eval_node_out(src, out, &mut cache, eval_subnet)
                .map(|r| r.into_mesh());
        }
        
        // Otherwise, evaluate the flagged node directly
        let mut cache = HashMap::new();
        self.eval_node(display_node_id, &mut cache, eval_subnet)
            .map(|r| r.into_mesh())
    }

    /// The node the viewport displays, resolved through Output to its source.
    pub fn display_source(&self) -> Option<NodeId> {
        let id   = self.get_viewport_node()?;
        let node = self.nodes.iter().find(|n| n.id == id)?;
        if matches!(node.node_type, NodeType::Output) {
            node.inputs.first()?.connected_output.map(|(src, _)| src)
        } else {
            Some(id)
        }
    }

    /// Clip produced by one node, if it produces animation. Clip operators
    /// never go through subnets, so no subnet evaluator is needed.
    pub fn eval_anim(&self, id: NodeId) -> Option<std::sync::Arc<crate::core::anim::AnimData>> {
        self.eval_anim_out(id, 0)
    }

    /// Clip on one output socket of a node.
    pub fn eval_anim_out(&self, id: NodeId, output: usize) -> Option<std::sync::Arc<crate::core::anim::AnimData>> {
        let passthrough = |_: SubnetId, mesh: &MeshData, _: Option<&MeshData>| mesh.clone();
        let mut cache = HashMap::new();
        match self.eval_anim_node(id, output, &mut cache, &passthrough)? {
            EvalResult::Anim(a) => Some(a),
            _ => None,
        }
    }

    /// Like `eval_node`, but stops as soon as a branch is known not to be
    /// animation, so asking a mesh node for its clip does not cook the mesh.
    fn eval_anim_node(
        &self,
        id:          NodeId,
        output:      usize,
        cache:       &mut EvalCache,
        eval_subnet: &impl Fn(SubnetId, &MeshData, Option<&MeshData>) -> MeshData,
    ) -> Option<EvalResult> {
        let node = self.nodes.iter().find(|n| n.id == id)?;
        match &node.node_type {
            t if t.is_anim() => self.eval_node_out(id, output, cache, eval_subnet),
            NodeType::Output => {
                let (src, out) = node.inputs.first()?.connected_output?;
                self.eval_anim_node(src, out, cache, eval_subnet)
            }
            _ => None,
        }
    }

    // Recursive bottom-up evaluator — follows the full chain.
    pub fn eval_node(
        &self,
        id:          NodeId,
        cache:       &mut EvalCache,
        eval_subnet: &impl Fn(SubnetId, &MeshData, Option<&MeshData>) -> MeshData,
    ) -> Option<EvalResult> {
        self.eval_node_out(id, 0, cache, eval_subnet)
    }

    /// Evaluate one output socket of a node. Each input is taken from the
    /// output socket it is wired to.
    pub fn eval_node_out(
        &self,
        id:          NodeId,
        output:      usize,
        cache:       &mut EvalCache,
        eval_subnet: &impl Fn(SubnetId, &MeshData, Option<&MeshData>) -> MeshData,
    ) -> Option<EvalResult> {
        if let Some(cached) = cache.get(&(id, output)) { return cached.clone(); }

        let node = self.nodes.iter().find(|n| n.id == id)?;

        let inputs: Vec<EvalResult> = node.inputs.iter()
            .filter_map(|s| s.connected_output
                .and_then(|(src, out)| self.eval_node_out(src, out, cache, eval_subnet)))
            .collect();

        let result = evaluate_node_type(&node.node_type, &inputs, eval_subnet, output);
        cache.insert((id, output), result.clone());
        result
    }

    // ── Scene explorer evaluation ─────────────────────────────────────────────
    // Separate from viewport eval. Walks the graph to collect generator nodes
    // (Cube, Sphere, LoadUsd…) with their names and raw EvalResults for the
    // scene hierarchy UI. Operators are transparent here — only leaf generators
    // appear as scene objects.
    pub fn evaluate_for_scene(
        &self,
        eval_subnet: &impl Fn(SubnetId, &MeshData, Option<&MeshData>) -> MeshData,
    ) -> Vec<(NodeId, String, EvalResult)> {
        let mut cache: EvalCache = HashMap::new();
        let mut out    = vec![];
        let mut visited = std::collections::HashSet::new();

        let root_src = self.nodes.iter()
            .find(|n| matches!(n.node_type, NodeType::Output))
            .and_then(|n| n.inputs.first())
            .and_then(|i| i.connected_output);

        // Only walk nodes reachable from Output
        if let Some((src, _)) = root_src {
            self.walk_for_scene(src, &mut cache, eval_subnet, &mut out, &mut visited);
        }

        // Animation: show the skeleton that reaches Output, after every
        // operator, so renames and splits are visible in the explorer.
        if let Some((src, socket)) = root_src {
            if let Some(r @ EvalResult::Anim(_)) = self.eval_node_out(src, socket, &mut cache, eval_subnet) {
                if let Some(n) = self.nodes.iter().find(|n| n.id == src) {
                    out.push((n.id, n.name.clone(), r));
                }
            }
        }
        out
    }

    fn walk_for_scene(
        &self,
        id:          NodeId,
        cache:       &mut EvalCache,
        eval_subnet: &impl Fn(SubnetId, &MeshData, Option<&MeshData>) -> MeshData,
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

        // Only mesh generator nodes appear in the scene explorer
        if matches!(node.node_type,
            NodeType::CreateCube { .. } | NodeType::CreateSphere { .. }
            | NodeType::CreateGrid { .. } | NodeType::LoadUsd { .. })
        {
            if let Some(r) = self.eval_node_out(id, 0, cache, eval_subnet) {
                out.push((node.id, node.name.clone(), r));
            }
        }
    }

    /// Clips the viewport should draw: the viewed node's clip, or one per
    /// output when the viewed node is a Split.
    pub fn display_clips(&self) -> Vec<std::sync::Arc<crate::core::anim::AnimData>> {
        let Some(id) = self.get_viewport_node() else { return vec![] };
        let Some(node) = self.nodes.iter().find(|n| n.id == id) else { return vec![] };
        match &node.node_type {
            NodeType::Output => node.inputs.first()
                .and_then(|i| i.connected_output)
                .and_then(|(src, out)| self.eval_anim_out(src, out))
                .into_iter().collect(),
            NodeType::SplitSkeleton { picks } =>
                (0..picks.len()).filter_map(|o| self.eval_anim_out(id, o)).collect(),
            _ => self.eval_anim(id).into_iter().collect(),
        }
    }

    /// Make a node's sockets match its type again after its parameters
    /// changed the socket count, and drop wires to sockets that are gone.
    pub fn sync_sockets(&mut self, id: NodeId) {
        let Some(node) = self.nodes.iter_mut().find(|n| n.id == id) else { return };
        let (_, outputs) = Self::create_sockets(&node.node_type);
        if outputs.len() == node.outputs.len() { return; }
        let count = outputs.len();
        node.outputs = outputs;
        let dead: Vec<ConnectionId> = self.connections.iter()
            .filter(|c| c.from_node == id && c.from_output >= count)
            .map(|c| c.id).collect();
        for cid in dead { self.remove_connection(cid); }
        self.mark_dirty();
    }
}