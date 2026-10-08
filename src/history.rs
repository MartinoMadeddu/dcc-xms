//! Undo history for the scene: the node graph and everything in it.
//!
//! What is undone is the document: nodes, their parameters, Edit Poly
//! operations, names, bypass, wires, node positions and the display flag.
//! The interface is not: camera, layout, theme, playhead, panning and which
//! node is selected never make a step. Undoing does select the node the step
//! changed, so the change is in front of you.
//!
//! A step is a snapshot of the document. Nodes that did not change are
//! shared with the step before, so a step costs only what changed. Nothing
//! has to know how to reverse itself: going back is putting an earlier
//! snapshot back, and the cooked results of that state are usually still in
//! the caches.
//!
//! Steps are made from gestures, not frames: while a mouse button is down or
//! a text field has the keyboard, changes gather; when it is let go they
//! become one step. Every frame the document is compared with the current
//! step, so no change can get past the history, whoever made it.
//!
//! Changing the selection of an Edit Poly node is not a step of its own: it
//! goes into the next real change, so "pick 12 polygons, extrude" is one step
//! and undoing it puts the old selection back too.
//!
//! Undoing and then changing something does not throw the undone steps
//! away: they stay as a branch, and the History pane can go back to them.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy_egui::egui;

use crate::node_graph::{Connection, GraphNode, NodeGraphState};
use crate::types::{NodeId, NodeType};

/// Memory the steps may take before the oldest are dropped.
pub const BUDGET: usize = 256 * 1024 * 1024;

// ============================================================================
// DOCUMENT
// ============================================================================

/// One state of the document.
#[derive(Clone)]
pub struct Doc {
    nodes:       Vec<Arc<GraphNode>>,
    /// Hash of each node, in the same order, to share unchanged nodes.
    node_hashes: Vec<u64>,
    connections: Vec<Connection>,
    view_flag:   Option<NodeId>,
    next_node:   usize,
    next_conn:   usize,
}

/// Bytes written are hashed instead of stored.
struct HashWriter<'a>(&'a mut std::collections::hash_map::DefaultHasher, usize);
impl std::io::Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> { self.0.write(buf); self.1 += buf.len(); Ok(buf.len()) }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

/// Hash of one node. `core` leaves out the Edit Poly selection being built
/// (and which operation's selection is being edited): changing those alone
/// is not a step. Also returns roughly how many bytes the node takes.
fn node_hash(n: &GraphNode, core: bool) -> (u64, usize) {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    n.id.0.hash(&mut h);
    n.name.hash(&mut h);
    n.bypassed.hash(&mut h);
    n.position.x.to_bits().hash(&mut h);
    n.position.y.to_bits().hash(&mut h);
    for i in &n.inputs { i.connected_output.map(|(id, out)| (id.0, out)).hash(&mut h); }
    n.outputs.len().hash(&mut h);
    let mut w = HashWriter(&mut h, 0);
    match &n.node_type {
        NodeType::EditPoly { ops, auto_collapse, .. } if core => {
            let _ = serde_json::to_writer(&mut w, ops);
            auto_collapse.hash(w.0);
        }
        t => { let _ = serde_json::to_writer(&mut w, t); }
    }
    let bytes = w.1 + n.name.len() + 64 * (1 + n.inputs.len() + n.outputs.len());
    (h.finish(), bytes)
}

/// Hash of the whole document, and of its core (see `node_hash`). Run every
/// frame, so only Edit Poly nodes are hashed twice.
fn doc_hashes(g: &NodeGraphState) -> (u64, u64) {
    let mut full = std::collections::hash_map::DefaultHasher::new();
    let mut core = std::collections::hash_map::DefaultHasher::new();
    for n in &g.nodes {
        let f = node_hash(n, false).0;
        f.hash(&mut full);
        let c = if matches!(n.node_type, NodeType::EditPoly { .. }) { node_hash(n, true).0 } else { f };
        c.hash(&mut core);
    }
    for h in [&mut full, &mut core] {
        for c in &g.connections { (c.from_node.0, c.from_output, c.to_node.0, c.to_input).hash(h); }
        g.view_flag.map(|n| n.0).hash(h);
    }
    (full.finish(), core.finish())
}

impl Doc {
    /// Snapshot of the graph. Nodes equal to one in `prev` are shared with it.
    /// Returns the bytes the new nodes take.
    fn capture(g: &NodeGraphState, prev: Option<&Doc>) -> (Doc, usize) {
        let shared: HashMap<u64, &Arc<GraphNode>> = prev
            .map(|p| p.node_hashes.iter().copied().zip(p.nodes.iter()).collect())
            .unwrap_or_default();
        let mut bytes = 0;
        let mut nodes = Vec::with_capacity(g.nodes.len());
        let mut node_hashes = Vec::with_capacity(g.nodes.len());
        for n in &g.nodes {
            let (h, size) = node_hash(n, false);
            match shared.get(&h) {
                Some(a) => nodes.push(Arc::clone(a)),
                None => { bytes += size; nodes.push(Arc::new(n.clone())); }
            }
            node_hashes.push(h);
        }
        bytes += g.connections.len() * std::mem::size_of::<Connection>() + 128;
        (Doc {
            nodes, node_hashes,
            connections: g.connections.clone(),
            view_flag:   g.view_flag,
            next_node:   g.next_node_id,
            next_conn:   g.next_connection_id,
        }, bytes)
    }

    /// Put this state back into the graph. Interactions in progress are
    /// dropped; ids keep counting up, so a node made after an undo never
    /// takes the id of one that was undone.
    fn restore(&self, g: &mut NodeGraphState) {
        g.nodes = self.nodes.iter().map(|n| (**n).clone()).collect();
        g.connections = self.connections.clone();
        g.view_flag = self.view_flag;
        g.next_node_id = g.next_node_id.max(self.next_node);
        g.next_connection_id = g.next_connection_id.max(self.next_conn);
        g.dragging_node = None;
        g.connecting_from = None;
        g.renaming_node = None;
        g.marquee_start = None;
        g.selected_connection = None;
        g.menu_from = None;
        g.tab_menu_screen_pos = None;
        g.tab_menu_canvas_pos = None;
        let exists = |id: &NodeId| g.nodes.iter().any(|n| n.id == *id);
        let keep: Vec<NodeId> = g.selected_nodes.iter().copied().filter(|id| exists(id)).collect();
        g.selected_nodes = keep;
        if g.selected_node.map_or(false, |id| !exists(&id)) { g.selected_node = None; }
        g.graph_version = g.graph_version.wrapping_add(1);
    }

    fn node(&self, id: NodeId) -> Option<&GraphNode> { self.nodes.iter().find(|n| n.id == id).map(|n| &**n) }
}

// ============================================================================
// STEPS
// ============================================================================

/// What kind of change a step is, for its icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Start, Load, Add, Delete, Wire, Edit, Rename, Bypass, Display, Move }

impl Kind {
    pub fn icon(self) -> &'static str {
        match self {
            Kind::Start   => "🏁",
            Kind::Load    => "📄",
            Kind::Add     => "➕",
            Kind::Delete  => "✖",
            Kind::Wire    => "🔗",
            Kind::Edit    => "✏",
            Kind::Rename  => "🏷",
            Kind::Bypass  => "⛔",
            Kind::Display => "👁",
            Kind::Move    => "✋",
        }
    }
}

pub struct Step {
    pub label:  String,
    pub kind:   Kind,
    /// Icon of the node an edit changed, else of the kind of change.
    pub icon:   &'static str,
    /// Nodes the step changed, added or removed.
    pub nodes:  Vec<NodeId>,
    pub when:   Instant,
    /// Kept whatever the memory budget.
    pub pinned: bool,
    doc:        Doc,
    full:       u64,
    core:       u64,
    bytes:      usize,
    parent:     Option<usize>,
    children:   Vec<usize>,
    /// Child that redo goes to: the one last visited or made.
    redo:       Option<usize>,
}

/// What the History pane or a key asked for, done by `update`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request { Undo, Redo, Jump(usize) }

#[derive(bevy::prelude::Resource)]
pub struct History {
    steps:   Vec<Option<Step>>,
    root:    usize,
    current: usize,
    bytes:   usize,
    budget:  usize,
    pub request: Option<Request>,
    /// History pane: list only the steps that touched the selected node.
    pub only_selected: bool,
    started: bool,
}

impl Default for History {
    fn default() -> Self { Self::with_budget(BUDGET) }
}

/// Name for the next step, set by whoever replaces the whole graph
/// (a template, an opened file). Used by the next step made.
static NOTE: Mutex<Option<(String, Kind)>> = Mutex::new(None);

/// Name the next step. For changes the difference alone cannot describe well.
pub fn note(label: impl Into<String>) {
    if let Ok(mut n) = NOTE.lock() { *n = Some((label.into(), Kind::Load)); }
}

fn take_note() -> Option<(String, Kind)> { NOTE.lock().ok().and_then(|mut n| n.take()) }

impl History {
    pub fn with_budget(budget: usize) -> Self {
        Self { steps: vec![], root: 0, current: 0, bytes: 0, budget, request: None, only_selected: false, started: false }
    }

    pub fn step(&self, i: usize) -> Option<&Step> { self.steps.get(i).and_then(|s| s.as_ref()) }
    fn step_mut(&mut self, i: usize) -> Option<&mut Step> { self.steps.get_mut(i).and_then(|s| s.as_mut()) }
    pub fn current(&self) -> usize { self.current }
    pub fn bytes(&self) -> usize { self.bytes }
    pub fn len(&self) -> usize { self.steps.iter().flatten().count() }
    pub fn can_undo(&self) -> bool { self.step(self.current).map_or(false, |s| s.parent.is_some()) }
    pub fn can_redo(&self) -> bool { self.redo_target().is_some() }

    fn redo_target(&self) -> Option<usize> {
        let s = self.step(self.current)?;
        s.redo.filter(|r| s.children.contains(r)).or_else(|| s.children.last().copied())
    }

    /// Start from the graph as it is. Done by the first `update`.
    pub fn start(&mut self, g: &NodeGraphState) {
        let (doc, bytes) = Doc::capture(g, None);
        let (full, core) = doc_hashes(g);
        self.steps = vec![Some(Step {
            label: "Start".into(), kind: Kind::Start, icon: Kind::Start.icon(), nodes: vec![], when: Instant::now(), pinned: false,
            doc, full, core, bytes, parent: None, children: vec![], redo: None,
        })];
        self.root = 0;
        self.current = 0;
        self.bytes = bytes;
        self.started = true;
        take_note();
    }

    /// Once a frame, after the interface has run. `busy`: a gesture is in
    /// progress (a mouse button down, a text field being typed in), so
    /// changes keep gathering. Returns true when the graph was changed by an
    /// undo, redo or jump.
    pub fn update(&mut self, g: &mut NodeGraphState, busy: bool) -> bool {
        if !self.started { self.start(g); return false; }
        if busy { return false; }
        self.commit(g);
        match self.request.take() {
            Some(Request::Undo) => self.undo(g),
            Some(Request::Redo) => self.redo(g),
            Some(Request::Jump(i)) => self.jump(g, i),
            None => false,
        }
    }

    /// Make a step if the document differs from the current one.
    /// Returns true if a step was made.
    pub fn commit(&mut self, g: &NodeGraphState) -> bool {
        let (full, core) = doc_hashes(g);
        let Some(head) = self.step(self.current) else { return false };
        if head.full == full || head.core == core {
            // Nothing, or only the Edit Poly selection: that goes into the
            // next real step.
            let _ = take_note();
            return false;
        }
        let (doc, bytes) = Doc::capture(g, Some(&head.doc));
        let (mut label, mut kind, nodes) = describe(&head.doc, &doc);
        if let Some((l, k)) = take_note() { label = l; kind = k; }
        let icon = match (kind, nodes.first().and_then(|id| doc.node(*id))) {
            (Kind::Edit, Some(n)) => crate::types::node_type_icon(&n.node_type),
            _ => kind.icon(),
        };
        let index = self.steps.len();
        let parent = self.current;
        self.steps.push(Some(Step {
            label, kind, icon, nodes, when: Instant::now(), pinned: false,
            doc, full, core, bytes, parent: Some(parent), children: vec![], redo: None,
        }));
        if let Some(p) = self.step_mut(parent) { p.children.push(index); p.redo = Some(index); }
        self.current = index;
        self.bytes += bytes;
        self.trim();
        true
    }

    pub fn undo(&mut self, g: &mut NodeGraphState) -> bool {
        let Some(from) = self.step(self.current) else { return false };
        let Some(parent) = from.parent else { return false };
        let focus = from.nodes.clone();
        let here = self.current;
        if let Some(p) = self.step_mut(parent) { p.redo = Some(here); }
        self.go(g, parent, &focus)
    }

    pub fn redo(&mut self, g: &mut NodeGraphState) -> bool {
        let Some(to) = self.redo_target() else { return false };
        let focus = self.step(to).map(|s| s.nodes.clone()).unwrap_or_default();
        self.go(g, to, &focus)
    }

    /// Go to any step, on any branch. Redo then follows the path taken.
    pub fn jump(&mut self, g: &mut NodeGraphState, to: usize) -> bool {
        if self.step(to).is_none() || to == self.current { return false; }
        let mut child = to;
        while let Some(p) = self.step(child).and_then(|s| s.parent) {
            if let Some(ps) = self.step_mut(p) { ps.redo = Some(child); }
            child = p;
        }
        let focus = self.step(to).map(|s| s.nodes.clone()).unwrap_or_default();
        self.go(g, to, &focus)
    }

    fn go(&mut self, g: &mut NodeGraphState, to: usize, focus: &[NodeId]) -> bool {
        let Some(step) = self.step(to) else { return false };
        step.doc.restore(g);
        self.current = to;
        // Show what changed: select the first node of the step that exists.
        if let Some(id) = focus.iter().copied().find(|id| g.nodes.iter().any(|n| n.id == *id)) {
            g.selected_node = Some(id);
            g.selected_nodes = vec![id];
        }
        true
    }

    pub fn set_pinned(&mut self, i: usize, pinned: bool) {
        if let Some(s) = self.step_mut(i) { s.pinned = pinned; }
    }

    /// Steps from the start to the end of the line redo would follow.
    pub fn line(&self) -> Vec<usize> {
        let mut path = vec![];
        let mut at = Some(self.current);
        while let Some(i) = at { path.push(i); at = self.step(i).and_then(|s| s.parent); }
        path.reverse();
        let mut at = self.current;
        loop {
            let Some(s) = self.step(at) else { break };
            let next = s.redo.filter(|r| s.children.contains(r)).or_else(|| s.children.last().copied());
            match next { Some(n) => { path.push(n); at = n; } None => break }
        }
        path
    }

    /// Branches that leave the line at step `i`: for each, its last step and
    /// how many steps it has.
    pub fn branches_at(&self, i: usize, line: &[usize]) -> Vec<(usize, usize)> {
        let Some(s) = self.step(i) else { return vec![] };
        s.children.iter().filter(|c| !line.contains(c)).map(|&c| {
            let (mut tip, mut n) = (c, 1);
            while let Some(&next) = self.step(tip).and_then(|t| t.children.last()) { tip = next; n += 1; }
            (tip, n)
        }).collect()
    }

    /// Drop the oldest steps until the history fits its budget. The current
    /// step and pinned steps always stay. Side branches go first, oldest
    /// first, then the oldest steps of the line. A step taken out of the
    /// middle of the line is skipped over: its neighbours are whole states
    /// of their own, so going from one to the other still works.
    fn trim(&mut self) {
        while self.bytes > self.budget {
            let on_line = self.line();
            let removable = |h: &History, i: usize| h.step(i).map_or(false, |s|
                i != h.current && !s.pinned && (s.parent.is_some() || s.children.len() == 1));
            let leaf = (0..self.steps.len()).find(|&i| removable(self, i) && !on_line.contains(&i)
                && self.step(i).map_or(false, |s| s.children.is_empty()));
            let pick = leaf.or_else(|| on_line.iter().copied().find(|&i| removable(self, i)));
            match pick { Some(i) => self.remove(i), None => break }
        }
    }

    /// Take a step out. Its children hang from its parent instead; the
    /// start's only child becomes the start.
    fn remove(&mut self, i: usize) {
        let Some(s) = self.steps.get_mut(i).and_then(|s| s.take()) else { return };
        self.bytes = self.bytes.saturating_sub(s.bytes);
        for &c in &s.children { if let Some(cs) = self.step_mut(c) { cs.parent = s.parent; } }
        match s.parent {
            Some(p) => if let Some(ps) = self.step_mut(p) {
                let at = ps.children.iter().position(|c| *c == i).unwrap_or(ps.children.len());
                ps.children.retain(|c| *c != i);
                for (k, &c) in s.children.iter().enumerate() { ps.children.insert((at + k).min(ps.children.len()), c); }
                if ps.redo == Some(i) { ps.redo = s.redo.or_else(|| s.children.last().copied()); }
            },
            None => if let Some(&c) = s.children.first() { self.root = c; },
        }
    }
}

// ============================================================================
// LABELS
// ============================================================================

/// Name, kind and nodes of the change from `a` to `b`.
fn describe(a: &Doc, b: &Doc) -> (String, Kind, Vec<NodeId>) {
    let name = |d: &Doc, id: NodeId| d.node(id).map(|n| n.name.clone()).unwrap_or_else(|| format!("node {}", id.0));
    let added: Vec<NodeId> = b.nodes.iter().map(|n| n.id).filter(|id| a.node(*id).is_none()).collect();
    let removed: Vec<NodeId> = a.nodes.iter().map(|n| n.id).filter(|id| b.node(*id).is_none()).collect();

    let wire = |c: &Connection| (c.from_node, c.from_output, c.to_node, c.to_input);
    let wa: Vec<_> = a.connections.iter().map(wire).collect();
    let wb: Vec<_> = b.connections.iter().map(wire).collect();
    let wired: Vec<_> = wb.iter().filter(|w| !wa.contains(w)).copied().collect();
    let unwired: Vec<_> = wa.iter().filter(|w| !wb.contains(w)).copied().collect();

    let mut params = vec![]; let mut renamed = vec![]; let mut bypassed = vec![]; let mut moved = vec![];
    for (i, n) in b.nodes.iter().enumerate() {
        let Some(o) = a.node(n.id) else { continue };
        if a.nodes.iter().position(|x| x.id == n.id).map(|j| a.node_hashes[j]) == Some(b.node_hashes[i]) { continue; }
        if node_type_value(&o.node_type, true) != node_type_value(&n.node_type, true) { params.push(n.id); }
        if o.name != n.name { renamed.push(n.id); }
        if o.bypassed != n.bypassed { bypassed.push(n.id); }
        if o.position != n.position { moved.push(n.id); }
    }

    let mut touched: Vec<NodeId> = vec![];
    let mut touch = |ids: &[NodeId]| for id in ids { if !touched.contains(id) { touched.push(*id); } };
    touch(&params); touch(&added); touch(&renamed); touch(&bypassed);
    let wire_nodes: Vec<NodeId> = wired.iter().chain(unwired.iter()).flat_map(|w| [w.2, w.0]).collect();
    touch(&wire_nodes); touch(&removed); touch(&moved);
    if a.view_flag != b.view_flag { touch(&b.view_flag.into_iter().collect::<Vec<_>>()); }

    let count = |n: usize, one: String, many: &str| if n == 1 { one } else { format!("{n} {many}") };
    let (label, kind) = if !added.is_empty() && !removed.is_empty() && removed.len() + added.len() > 2 {
        ("Replace the graph".into(), Kind::Load)
    } else if !added.is_empty() {
        (format!("Add {}", count(added.len(), name(b, added[0]), "nodes")), Kind::Add)
    } else if !removed.is_empty() {
        (format!("Delete {}", count(removed.len(), name(a, removed[0]), "nodes")), Kind::Delete)
    } else if !wired.is_empty() || !unwired.is_empty() {
        let text = |d: &Doc, w: &(NodeId, usize, NodeId, usize)| format!("{} to {}", name(d, w.0), name(d, w.2));
        let l = match (wired.len(), unwired.len()) {
            (1, 0) => format!("Connect {}", text(b, &wired[0])),
            (0, 1) => format!("Disconnect {}", text(a, &unwired[0])),
            (1, 1) => format!("Rewire {}", text(b, &wired[0])),
            (x, y) => format!("Change {} wires", x + y),
        };
        (l, Kind::Wire)
    } else if !params.is_empty() {
        let l = if params.len() == 1 {
            let id = params[0];
            let what = param_change(&a.node(id).unwrap().node_type, &b.node(id).unwrap().node_type);
            format!("{}: {what}", name(b, id))
        } else {
            format!("Change {} nodes", params.len())
        };
        (l, Kind::Edit)
    } else if !renamed.is_empty() {
        (format!("Rename {} to {}", name(a, renamed[0]), name(b, renamed[0])), Kind::Rename)
    } else if !bypassed.is_empty() {
        let on = b.node(bypassed[0]).map_or(false, |n| n.bypassed);
        (format!("{} {}", if on { "Bypass" } else { "Turn on" }, count(bypassed.len(), name(b, bypassed[0]), "nodes")), Kind::Bypass)
    } else if a.view_flag != b.view_flag {
        match b.view_flag {
            Some(id) => (format!("Show {} in the viewport", name(b, id)), Kind::Display),
            None     => ("Show Output in the viewport".into(), Kind::Display),
        }
    } else if !moved.is_empty() {
        (format!("Move {}", count(moved.len(), name(b, moved[0]), "nodes")), Kind::Move)
    } else {
        ("Change".into(), Kind::Edit)
    };
    (label, kind, touched)
}

/// Parameters as JSON, without the Edit Poly selection being built when
/// `core` is set.
fn node_type_value(t: &NodeType, core: bool) -> serde_json::Value {
    let mut v = serde_json::to_value(t).unwrap_or(serde_json::Value::Null);
    if core {
        if let Some(e) = v.get_mut("EditPoly").and_then(|e| e.as_object_mut()) {
            e.remove("pending");
            e.remove("edit");
        }
    }
    v
}

/// "#6 Bevel height 0.12 to 0.15", "add Extrude", "size 1 to 2".
fn param_change(a: &NodeType, b: &NodeType) -> String {
    let (va, vb) = (node_type_value(a, true), node_type_value(b, true));
    // Inside the variant: {"CreateCube": {...}}.
    let (ia, ib) = match (&va, &vb) {
        (serde_json::Value::Object(x), serde_json::Value::Object(y)) if x.len() == 1 && y.len() == 1 => {
            (x.values().next().unwrap(), y.values().next().unwrap())
        }
        _ => (&va, &vb),
    };
    // Edit Poly: operations added, removed or reordered.
    if let (Some(oa), Some(ob)) = (ia.get("ops").and_then(|o| o.as_array()), ib.get("ops").and_then(|o| o.as_array())) {
        if ob.len() == oa.len() + 1 && ob[..oa.len()] == oa[..] {
            return format!("add {}", op_kind(&ob[oa.len()]));
        }
        if oa.len() == ob.len() + 1 {
            if let Some(i) = (0..oa.len()).find(|&i| { let mut c = oa.clone(); c.remove(i); c[..] == ob[..] }) {
                return format!("delete #{} {}", i + 1, op_kind(&oa[i]));
            }
        }
        if oa.len() != ob.len() { return "change operations".into(); }
        let mut sa = oa.clone(); let mut sb = ob.clone();
        sa.sort_by_key(|v| v.to_string()); sb.sort_by_key(|v| v.to_string());
        if sa == sb && oa != ob { return "reorder operations".into(); }
    }
    let mut diffs = vec![];
    leaf_diffs(ia, ib, &mut vec![], &mut diffs);
    match diffs.len() {
        0 => "change".into(),
        1 => {
            let (path, x, y) = &diffs[0];
            let where_ = render_path(ib, path);
            match (x, y) {
                (Some(_), Some(y)) if y.is_boolean() && path.last().map_or(false, |p| p == "collapsed") =>
                    format!("{} {}", if y.as_bool() == Some(true) { "collapse" } else { "restore" }, where_.trim_end_matches(" collapsed")),
                (Some(_), Some(y)) if y.is_boolean() && path.last().map_or(false, |p| p == "enabled") =>
                    format!("{} {}", if y.as_bool() == Some(true) { "turn on" } else { "turn off" }, where_.trim_end_matches(" enabled")),
                (Some(x), Some(y)) => format!("{where_} {} to {}", show(x), show(y)).trim().to_string(),
                _ => format!("{where_} changed").trim().to_string(),
            }
        }
        n => {
            // The same switch on several operations: "restore 5 operations".
            let last = diffs[0].0.last().cloned().unwrap_or_default();
            let same = diffs.iter().all(|d| d.0.last() == Some(&last) && d.2 == diffs[0].2);
            if same && (last == "collapsed" || last == "enabled") {
                let on = diffs[0].2.as_ref().and_then(|v| v.as_bool()) == Some(true);
                let verb = match (last.as_str(), on) {
                    ("collapsed", true) => "collapse", ("collapsed", false) => "restore",
                    (_, true) => "turn on", _ => "turn off",
                };
                return format!("{verb} {n} operations");
            }
            // Common start of the paths.
            let first = &diffs[0].0;
            let common = (0..first.len()).take_while(|&k| diffs.iter().all(|d| d.0.get(k) == first.get(k))).count();
            let where_ = render_path(ib, &first[..common]);
            if where_.is_empty() { format!("{n} values") } else { format!("{where_} ({n} values)") }
        }
    }
}

fn op_kind(op: &serde_json::Value) -> String {
    match op.get("kind") {
        Some(serde_json::Value::Object(o)) => o.keys().next().cloned().unwrap_or_default(),
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => "operation".into(),
    }
}

type Diff = (Vec<String>, Option<serde_json::Value>, Option<serde_json::Value>);

fn leaf_diffs(a: &serde_json::Value, b: &serde_json::Value, path: &mut Vec<String>, out: &mut Vec<Diff>) {
    use serde_json::Value::*;
    if a == b { return; }
    match (a, b) {
        (Object(x), Object(y)) if x.len() == y.len() && x.keys().eq(y.keys()) => {
            for (k, va) in x {
                path.push(k.clone());
                leaf_diffs(va, &y[k], path, out);
                path.pop();
            }
        }
        (Array(x), Array(y)) if x.len() == y.len() => {
            for (i, (va, vb)) in x.iter().zip(y).enumerate() {
                path.push(i.to_string());
                leaf_diffs(va, vb, path, out);
                path.pop();
            }
        }
        (Object(_), Object(_)) | (Array(_), Array(_)) => out.push((path.clone(), None, None)),
        _ => out.push((path.clone(), Some(a.clone()), Some(b.clone()))),
    }
}

/// Path inside the parameters as words: "#6 Bevel height", "translate x".
fn render_path(b: &serde_json::Value, path: &[String]) -> String {
    let mut words: Vec<String> = vec![];
    let mut at = Some(b);
    let mut parent_key: Option<&str> = None;
    for part in path {
        let next = at.and_then(|v| match v {
            serde_json::Value::Object(o) => o.get(part),
            serde_json::Value::Array(x) => part.parse::<usize>().ok().and_then(|i| x.get(i)),
            _ => None,
        });
        match at {
            Some(serde_json::Value::Array(x)) => {
                let i: usize = part.parse().unwrap_or(0);
                if parent_key == Some("ops") {
                    words.push(format!("#{} {}", i + 1, next.map(op_kind).unwrap_or_default()));
                } else if x.len() <= 4 && x.iter().all(|v| v.is_number()) {
                    words.push(["x", "y", "z", "w"][i.min(3)].into());
                } else {
                    words.push(format!("#{}", i + 1));
                }
            }
            // The operation kind is already named with its number.
            _ => if !matches!(part.as_str(), "kind" | "ops") && parent_key != Some("kind") { words.push(part.replace('_', " ")); },
        }
        parent_key = Some(part.as_str());
        at = next;
    }
    words.join(" ")
}

fn show(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            if f.fract() == 0.0 && f.abs() < 1e9 { format!("{}", f as i64) } else {
                let s = format!("{f:.3}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
        serde_json::Value::Bool(b) => if *b { "on".into() } else { "off".into() },
        serde_json::Value::String(s) => {
            let short = s.rsplit(['/', '\\']).next().unwrap_or(s);
            if short.is_empty() { "\"\"".into() } else if short.chars().count() > 32 {
                format!("\"{}…\"", short.chars().take(30).collect::<String>())
            } else { format!("\"{short}\"") }
        }
        other => {
            let s = other.to_string();
            if s.len() > 24 { "…".into() } else { s }
        }
    }
}

// ============================================================================
// INTERFACE
// ============================================================================

fn age(when: Instant) -> String {
    let s = when.elapsed().as_secs();
    if s < 5 { "now".into() } else if s < 60 { format!("{s} s") } else if s < 3600 { format!("{} min", s / 60) } else { format!("{} h", s / 3600) }
}

fn megabytes(b: usize) -> String {
    if b < 1024 * 1024 { format!("{} KB", (b + 1023) / 1024) } else { format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)) }
}

#[derive(Clone, Copy, PartialEq)]
enum Look { Done, Current, Ahead, Branch }

/// One row of the History pane: icon, label wrapped to the width, age.
fn row(ui: &mut egui::Ui, indent: f32, icon: &str, label: &str, when: &str, look: Look) -> egui::Response {
    let font = if look == Look::Branch { egui::FontId::proportional(12.0) } else { egui::FontId::proportional(13.0) };
    let v = ui.visuals().clone();
    let ink = match look {
        Look::Current => v.selection.stroke.color,
        Look::Done    => v.text_color(),
        Look::Ahead | Look::Branch => v.weak_text_color(),
    };
    let (icon_w, when_w, pad) = (22.0, 40.0, 3.0);
    let width = ui.available_width();
    let text_w = (width - indent - icon_w - when_w - 6.0).max(40.0);
    let galley = ui.painter().layout(label.to_string(), font.clone(), ink, text_w);
    let height = (galley.size().y + 2.0 * pad).max(20.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    if look == Look::Current {
        ui.painter().rect_filled(rect, 2.0, v.selection.bg_fill);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, v.widgets.hovered.weak_bg_fill);
    }
    let top = rect.top() + pad;
    let first_line = top + galley.rows.first().map_or(galley.size().y, |r| r.rect.height()) / 2.0;
    ui.painter().text(egui::pos2(rect.left() + indent + 4.0, first_line), egui::Align2::LEFT_CENTER, icon, font.clone(), ink);
    ui.painter().galley(egui::pos2(rect.left() + indent + icon_w, top), galley, ink);
    ui.painter().text(egui::pos2(rect.right() - 4.0, first_line), egui::Align2::RIGHT_CENTER, when,
        egui::FontId::proportional(11.0), if look == Look::Current { ink } else { v.weak_text_color() });
    resp
}

/// The History pane: every step from the start, the current one marked,
/// the ones redo would bring back dimmed. Click a step to go to it.
pub fn draw(ui: &mut egui::Ui, history: &mut History, selected: Option<NodeId>) {
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(history.can_undo(), egui::Button::new("⟲ Undo")).on_hover_text("Ctrl+Z").clicked() {
            history.request = Some(Request::Undo);
        }
        if ui.add_enabled(history.can_redo(), egui::Button::new("⟳ Redo")).on_hover_text("Ctrl+Shift+Z or Ctrl+Y").clicked() {
            history.request = Some(Request::Redo);
        }
        ui.checkbox(&mut history.only_selected, "Selected node")
            .on_hover_text("List only the steps that changed the selected node");
    });
    ui.label(egui::RichText::new(format!("{} steps, {}", history.len(), megabytes(history.bytes()))).small().weak());
    ui.separator();

    let line = history.line();
    let current = history.current();
    let filter = if history.only_selected { selected } else { None };
    let mut clicked = None;
    let mut pin = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        let mut ahead = false;
        for &i in &line {
            let Some(s) = history.step(i) else { continue };
            let shown = filter.map_or(true, |id| s.nodes.contains(&id) || i == current);
            if shown {
                let label = if s.pinned { format!("📌 {}", s.label) } else { s.label.clone() };
                let look = if i == current { Look::Current } else if ahead { Look::Ahead } else { Look::Done };
                let resp = row(ui, 0.0, s.icon, &label, &age(s.when), look)
                    .on_hover_text("Click to go to this step. Right-click to pin it, so it is never dropped");
                if resp.clicked() { clicked = Some(i); }
                resp.context_menu(|ui| {
                    let text = if s.pinned { "Unpin" } else { "Pin: keep this step" };
                    if ui.button(text).clicked() { pin = Some((i, !s.pinned)); ui.close_menu(); }
                });
            }
            // Branches that leave the line here, folded to one row each.
            for (tip, n) in history.branches_at(i, &line) {
                let Some(t) = history.step(tip) else { continue };
                if filter.map_or(false, |id| !t.nodes.contains(&id)) { continue; }
                let text = format!("{n} step{} on another branch, last: {}", if n == 1 { "" } else { "s" }, t.label);
                let resp = row(ui, 14.0, "🔀", &text, &age(t.when), Look::Branch)
                    .on_hover_text("Click to go to the end of that branch");
                if resp.clicked() { clicked = Some(tip); }
            }
            if i == current { ahead = true; }
        }
    });
    if let Some(i) = clicked { history.request = Some(Request::Jump(i)); }
    if let Some((i, p)) = pin { history.set_pinned(i, p); }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::poly::{PolyOp, PolyOpKind, PolySelection, ExtrudeMode};

    fn graph() -> NodeGraphState { NodeGraphState::default() }

    fn cube(g: &mut NodeGraphState, size: f32) -> NodeId {
        g.add_node("Cube".into(), NodeType::CreateCube { size }, egui::pos2(0.0, 0.0))
    }

    fn output(g: &NodeGraphState) -> NodeId {
        g.nodes.iter().find(|n| matches!(n.node_type, NodeType::Output)).unwrap().id
    }

    fn set_size(g: &mut NodeGraphState, id: NodeId, v: f32) {
        if let Some(NodeType::CreateCube { size }) = g.nodes.iter_mut().find(|n| n.id == id).map(|n| &mut n.node_type) { *size = v; }
    }

    fn label(h: &History) -> String { h.step(h.current()).unwrap().label.clone() }

    #[test]
    fn steps_are_named_from_what_changed() {
        let mut g = graph();
        let mut h = History::default();
        h.update(&mut g, false);
        let c = cube(&mut g, 1.0);
        h.update(&mut g, false);
        assert_eq!(label(&h), "Add Cube");
        set_size(&mut g, c, 2.5);
        h.update(&mut g, false);
        assert_eq!(label(&h), "Cube: size 1 to 2.5");
        let out = output(&g);
        g.add_connection(c, 0, out, 0);
        h.update(&mut g, false);
        assert_eq!(label(&h), "Connect Cube to Output");
        g.nodes.iter_mut().find(|n| n.id == c).unwrap().name = "Box".into();
        h.update(&mut g, false);
        assert_eq!(label(&h), "Rename Cube to Box");
        g.nodes.iter_mut().find(|n| n.id == c).unwrap().position = egui::pos2(40.0, 10.0);
        h.update(&mut g, false);
        assert_eq!(label(&h), "Move Box");
        assert_eq!(h.step(h.current()).unwrap().kind, Kind::Move);
    }

    #[test]
    fn a_gesture_is_one_step() {
        let mut g = graph();
        let mut h = History::default();
        h.update(&mut g, false);
        let c = cube(&mut g, 1.0);
        h.update(&mut g, false);
        let before = h.len();
        // A slider dragged over many frames.
        for k in 0..50 { set_size(&mut g, c, 1.0 + k as f32 * 0.1); h.update(&mut g, true); }
        assert_eq!(h.len(), before);
        h.update(&mut g, false);
        assert_eq!(h.len(), before + 1);
        assert_eq!(label(&h), "Cube: size 1 to 5.9");
    }

    #[test]
    fn undo_and_redo_give_back_each_state() {
        let mut g = graph();
        let mut h = History::default();
        h.update(&mut g, false);
        let start = doc_hashes(&g).0;
        let c = cube(&mut g, 1.0);
        h.update(&mut g, false);
        let one = doc_hashes(&g).0;
        set_size(&mut g, c, 3.0);
        h.update(&mut g, false);
        let two = doc_hashes(&g).0;

        assert!(h.undo(&mut g));
        assert_eq!(doc_hashes(&g).0, one);
        assert_eq!(g.selected_node, Some(c), "undo selects the node it changed");
        assert!(h.undo(&mut g));
        assert_eq!(doc_hashes(&g).0, start);
        assert!(!h.undo(&mut g));
        assert!(h.redo(&mut g));
        assert!(h.redo(&mut g));
        assert_eq!(doc_hashes(&g).0, two);
        assert!(!h.redo(&mut g));
        // Restoring makes no step of its own.
        let n = h.len();
        h.update(&mut g, false);
        assert_eq!(h.len(), n);
    }

    #[test]
    fn keys_and_buttons_wait_for_the_gesture_to_end() {
        let mut g = graph();
        let mut h = History::default();
        h.update(&mut g, false);
        cube(&mut g, 1.0);
        h.update(&mut g, false);
        h.request = Some(Request::Undo);
        assert!(!h.update(&mut g, true));
        assert_eq!(g.nodes.len(), 2);
        assert!(h.update(&mut g, false));
        assert_eq!(g.nodes.len(), 1);
    }

    #[test]
    fn a_change_not_yet_a_step_is_made_one_before_undo() {
        let mut g = graph();
        let mut h = History::default();
        h.update(&mut g, false);
        let c = cube(&mut g, 1.0);
        h.update(&mut g, false);
        set_size(&mut g, c, 9.0);
        h.request = Some(Request::Undo);
        h.update(&mut g, false);
        // The size change became a step, and that is what was undone.
        assert!(g.nodes.iter().any(|n| n.id == c));
        assert!(h.can_redo());
        h.redo(&mut g);
        assert!(matches!(g.nodes.iter().find(|n| n.id == c).unwrap().node_type, NodeType::CreateCube { size } if size == 9.0));
    }

    #[test]
    fn undone_steps_are_kept_as_a_branch() {
        let mut g = graph();
        let mut h = History::default();
        h.update(&mut g, false);
        let c = cube(&mut g, 1.0);
        h.update(&mut g, false);
        set_size(&mut g, c, 2.0); h.update(&mut g, false);
        set_size(&mut g, c, 3.0); h.update(&mut g, false);
        let old_tip = h.current();
        let old = doc_hashes(&g).0;
        h.undo(&mut g); h.undo(&mut g);
        set_size(&mut g, c, 7.0); h.update(&mut g, false);
        let line = h.line();
        let branch_point = line[line.len() - 2];
        let branches = h.branches_at(branch_point, &line);
        assert_eq!(branches, vec![(old_tip, 2)]);
        assert!(h.jump(&mut g, old_tip));
        assert_eq!(doc_hashes(&g).0, old);
        // Redo now follows the old branch.
        h.undo(&mut g); h.undo(&mut g);
        h.redo(&mut g); h.redo(&mut g);
        assert_eq!(h.current(), old_tip);
    }

    #[test]
    fn ids_are_never_taken_again_after_undo() {
        let mut g = graph();
        let mut h = History::default();
        h.update(&mut g, false);
        let a = cube(&mut g, 1.0);
        h.update(&mut g, false);
        h.undo(&mut g);
        let b = cube(&mut g, 1.0);
        assert_ne!(a, b);
    }

    fn edit_poly(g: &mut NodeGraphState, id: NodeId, f: impl FnOnce(&mut Vec<PolyOp>, &mut PolySelection)) {
        if let NodeType::EditPoly { ops, pending, .. } = &mut g.nodes.iter_mut().find(|n| n.id == id).unwrap().node_type {
            f(ops, pending);
        }
    }

    #[test]
    fn edit_poly_selection_goes_into_the_next_operation() {
        let mut g = graph();
        let mut h = History::default();
        h.update(&mut g, false);
        let e = g.add_node("Mine".into(), NodeType::EditPoly {
            ops: vec![], pending: PolySelection::default(), edit: None, auto_collapse: false,
        }, egui::pos2(0.0, 0.0));
        h.update(&mut g, false);
        let steps = h.len();
        // Pick polygons: no step.
        edit_poly(&mut g, e, |_, pending| pending.polys = vec![1, 2, 3]);
        h.update(&mut g, false);
        assert_eq!(h.len(), steps);
        // Extrude them: one step, named after the operation.
        edit_poly(&mut g, e, |ops, pending| {
            ops.push(PolyOp::new(pending.clone(), PolyOpKind::Extrude { height: 0.2, mode: ExtrudeMode::Group }));
            pending.polys.clear();
        });
        h.update(&mut g, false);
        assert_eq!(h.len(), steps + 1);
        assert_eq!(label(&h), "Mine: add Extrude");
        // Undo takes the operation away, and the pick with it.
        h.undo(&mut g);
        edit_poly(&mut g, e, |ops, pending| assert!(ops.is_empty() && pending.polys.is_empty()));
        // Changing an operation is named after it.
        h.redo(&mut g);
        edit_poly(&mut g, e, |ops, _| if let PolyOpKind::Extrude { height, .. } = &mut ops[0].kind { *height = 0.5; });
        h.update(&mut g, false);
        assert_eq!(label(&h), "Mine: #1 Extrude height 0.2 to 0.5");
        edit_poly(&mut g, e, |ops, _| ops[0].collapsed = true);
        h.update(&mut g, false);
        assert_eq!(label(&h), "Mine: collapse #1 Extrude");
        edit_poly(&mut g, e, |ops, pending| {
            ops.push(PolyOp::new(pending.clone(), PolyOpKind::Extrude { height: 0.1, mode: ExtrudeMode::Group }));
        });
        h.update(&mut g, false);
        edit_poly(&mut g, e, |ops, _| for o in ops.iter_mut() { o.collapsed = !o.collapsed; });
        h.update(&mut g, false);
        assert_eq!(label(&h), "Mine: 2 values");
        edit_poly(&mut g, e, |ops, _| for o in ops.iter_mut() { o.collapsed = true; });
        h.update(&mut g, false);
        edit_poly(&mut g, e, |ops, _| for o in ops.iter_mut() { o.collapsed = false; });
        h.update(&mut g, false);
        assert_eq!(label(&h), "Mine: restore 2 operations");
        assert_eq!(h.step(h.current()).unwrap().icon, crate::types::node_type_icon(&g.nodes.iter().find(|n| n.id == e).unwrap().node_type));
    }

    #[test]
    fn unchanged_nodes_are_shared_between_steps() {
        let mut g = graph();
        let mut h = History::default();
        h.update(&mut g, false);
        let ids: Vec<NodeId> = (0..20).map(|_| cube(&mut g, 1.0)).collect();
        h.update(&mut g, false);
        let before = h.bytes();
        set_size(&mut g, ids[3], 2.0);
        h.update(&mut g, false);
        let step = h.bytes() - before;
        let a = &h.step(h.current()).unwrap().doc;
        let b = &h.step(h.step(h.current()).unwrap().parent.unwrap()).unwrap().doc;
        let shared = a.nodes.iter().zip(&b.nodes).filter(|(x, y)| Arc::ptr_eq(x, y)).count();
        assert_eq!(shared, a.nodes.len() - 1);
        assert!(step < before / 4, "a one-node step takes {step} bytes, the graph {before}");
    }

    #[test]
    fn the_budget_drops_old_steps_but_not_the_current_or_pinned_ones() {
        let mut g = graph();
        let mut h = History::with_budget(40_000);
        h.update(&mut g, false);
        let c = cube(&mut g, 1.0);
        h.update(&mut g, false);
        let pinned = h.current();
        h.set_pinned(pinned, true);
        for k in 0..400 { set_size(&mut g, c, 2.0 + k as f32); h.update(&mut g, false); }
        assert!(h.bytes() <= 40_000, "{} bytes", h.bytes());
        assert!(h.len() < 400);
        assert!(h.step(h.current()).is_some());
        assert!(h.step(pinned).is_some(), "a pinned step stays");
        // Undo all the way back still works, through the pinned step.
        let mut seen_pinned = false;
        while h.undo(&mut g) { seen_pinned |= h.current() == pinned; }
        assert!(seen_pinned);
        assert_eq!(h.line().first().copied(), Some(h.current()));
    }

    /// Random edits, then undo everything: the graph is exactly as it began.
    /// Redo everything: exactly as it ended.
    #[test]
    fn random_edits_undo_and_redo_exactly() {
        let mut seed = 0x2545F4914F6CDD1Du64;
        let mut rnd = move |n: usize| { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; (seed % n as u64) as usize };
        for _round in 0..20 {
            let mut g = graph();
            let mut h = History::default();
            cube(&mut g, 1.0);
            h.update(&mut g, false);
            let start = doc_hashes(&g).0;
            let mut states = vec![start];
            for _ in 0..60 {
                let ids: Vec<NodeId> = g.nodes.iter().filter(|n| !matches!(n.node_type, NodeType::Output)).map(|n| n.id).collect();
                match rnd(7) {
                    0 => { cube(&mut g, rnd(10) as f32); }
                    1 if !ids.is_empty() => { g.selected_nodes = vec![ids[rnd(ids.len())]]; g.delete_selected(); }
                    2 if !ids.is_empty() => { let id = ids[rnd(ids.len())]; set_size(&mut g, id, rnd(100) as f32 * 0.5); }
                    3 if ids.len() >= 1 => { let out = output(&g); g.add_connection(ids[rnd(ids.len())], 0, out, 0); }
                    4 if !ids.is_empty() => { let id = ids[rnd(ids.len())]; if let Some(n) = g.nodes.iter_mut().find(|n| n.id == id) { n.position.x += 10.0; } }
                    5 if !ids.is_empty() => { let id = ids[rnd(ids.len())]; if let Some(n) = g.nodes.iter_mut().find(|n| n.id == id) { n.bypassed = !n.bypassed; } }
                    6 if !ids.is_empty() => { g.view_flag = Some(ids[rnd(ids.len())]); }
                    _ => {}
                }
                // Sometimes several changes make one gesture.
                if rnd(3) > 0 && h.update(&mut g, false) == false {
                    let now = doc_hashes(&g).0;
                    if *states.last().unwrap() != now { states.push(now); }
                }
            }
            h.update(&mut g, false);
            let now = doc_hashes(&g).0;
            if *states.last().unwrap() != now { states.push(now); }
            let end = now;
            let mut back = vec![doc_hashes(&g).0];
            while h.undo(&mut g) { back.push(doc_hashes(&g).0); }
            assert_eq!(doc_hashes(&g).0, start);
            back.reverse();
            assert_eq!(back, states, "every step undone in order");
            while h.redo(&mut g) {}
            assert_eq!(doc_hashes(&g).0, end);
        }
    }
}
