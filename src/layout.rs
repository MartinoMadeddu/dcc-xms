//! Dockable panes: which panes exist, where they sit, and the layout file.
//!
//! Every pane is a tab of one dock area. Tabs can be dragged to another
//! place, stacked, or pulled out as floating windows. The layout is kept in
//! the config folder and restored at the next start.

use bevy::prelude::Resource;
use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pane { Viewport, NodeGraph, SceneExplorer, OperatorStack, Properties, PrimInspector, Timeline, UvEditor }

impl Pane {
    pub const ALL: [Pane; 8] = [
        Pane::Viewport, Pane::NodeGraph, Pane::SceneExplorer, Pane::OperatorStack,
        Pane::Properties, Pane::PrimInspector, Pane::Timeline, Pane::UvEditor,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Pane::Viewport      => "Viewport",
            Pane::NodeGraph     => "Node Graph",
            Pane::SceneExplorer => "Scene Explorer",
            Pane::OperatorStack => "Operator Stack",
            Pane::Properties    => "Properties",
            Pane::PrimInspector => "Primitive Inspector",
            Pane::Timeline      => "Timeline",
            Pane::UvEditor      => "UV Editor",
        }
    }
}

/// A side of the program window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge { Left, Right, Top, Bottom }

impl Edge {
    /// The edge within `reach` of a point, if any. The nearest one wins.
    pub fn near(screen: bevy_egui::egui::Rect, p: bevy_egui::egui::Pos2, reach: f32) -> Option<Edge> {
        [
            (Edge::Left, p.x - screen.left()), (Edge::Right, screen.right() - p.x),
            (Edge::Top, p.y - screen.top()), (Edge::Bottom, screen.bottom() - p.y),
        ].into_iter().filter(|(_, d)| *d <= reach).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(e, _)| e)
    }

    /// Share of the window a pane docked on this edge starts with.
    pub fn share(self) -> f32 {
        match self { Edge::Left | Edge::Right => 0.22, Edge::Top | Edge::Bottom => 0.16 }
    }

    /// The strip of the window a pane docked on this edge will take.
    pub fn strip(self, screen: bevy_egui::egui::Rect) -> bevy_egui::egui::Rect {
        let (w, h) = (screen.width() * self.share(), screen.height() * self.share());
        let mut r = screen;
        match self {
            Edge::Left   => r.set_right(screen.left() + w),
            Edge::Right  => r.set_left(screen.right() - w),
            Edge::Top    => r.set_bottom(screen.top() + h),
            Edge::Bottom => r.set_top(screen.bottom() - h),
        }
        r
    }
}

#[derive(Resource)]
pub struct Layout {
    pub dock:   DockState<Pane>,
    /// Locked: tabs cannot be moved, floated or closed. Dividers still resize.
    pub locked: bool,
    /// The layout as last written to disk.
    saved:      String,
    /// Floating windows that have been given their starting size.
    sized:      Vec<usize>,
    /// Pane whose tab the mouse went down on, and whether it has moved since.
    pub tab_drag: Option<(Pane, bool)>,
}

/// Viewport on the left with the inspector under it, then the node graph,
/// the scene and stack column, and the properties. Timeline along the bottom.
/// The UV editor is a tab behind the node graph.
pub fn default_dock() -> DockState<Pane> {
    let mut dock = DockState::new(vec![Pane::Viewport]);
    let s = dock.main_surface_mut();
    let [top, _timeline] = s.split_below(NodeIndex::root(), 0.9, vec![Pane::Timeline]);
    let [viewport, rest] = s.split_right(top, 0.37, vec![Pane::NodeGraph]);
    let [_graph, rest]   = s.split_right(rest, 0.57, vec![Pane::SceneExplorer]);
    let [scene, _props]  = s.split_right(rest, 0.5, vec![Pane::Properties]);
    s.split_below(scene, 0.5, vec![Pane::OperatorStack]);
    s.split_below(viewport, 0.72, vec![Pane::PrimInspector]);
    // The UV editor shares the node graph's place, as a second tab behind it.
    if let Some((surface, node, _)) = dock.find_tab(&Pane::NodeGraph) {
        dock[surface][node].append_tab(Pane::UvEditor);
        dock.set_active_tab((surface, node, egui_dock::TabIndex(0)));
    }
    dock
}

/// A layout is usable when no pane is in it twice. Panes that are missing
/// are closed, and can be shown again from the Panes menu.
pub fn is_valid(dock: &DockState<Pane>) -> bool {
    let tabs: Vec<Pane> = dock.iter_all_tabs().map(|(_, t)| *t).collect();
    Pane::ALL.iter().all(|p| tabs.iter().filter(|t| *t == p).count() <= 1)
}

/// The layout as text. Rectangles of panes that have not been laid out yet
/// are infinite, which JSON cannot hold: they are written as zero. They are
/// worked out again on the first frame anyway.
pub fn to_json(dock: &DockState<Pane>, locked: bool) -> Option<String> {
    fn fix(v: &mut serde_json::Value, in_rect: bool) {
        match v {
            serde_json::Value::Null if in_rect => *v = serde_json::json!(0.0),
            serde_json::Value::Array(a) => for x in a { fix(x, in_rect); },
            serde_json::Value::Object(o) => for (k, x) in o.iter_mut() {
                fix(x, in_rect || k == "rect" || k == "viewport");
            },
            _ => {}
        }
    }
    let mut value = serde_json::to_value(dock).ok()?;
    fix(&mut value, false);
    serde_json::to_string(&serde_json::json!({ "locked": locked, "dock": value })).ok()
}

/// Layout and lock state from text. Files written before the lock existed
/// hold the layout alone.
pub fn from_json(json: &str) -> Option<(DockState<Pane>, bool)> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let (dock, locked) = match value.get("dock") {
        Some(d) => (d.clone(), value.get("locked").and_then(|l| l.as_bool()).unwrap_or(false)),
        None    => (value, false),
    };
    let dock: DockState<Pane> = serde_json::from_value(dock).ok()?;
    is_valid(&dock).then_some((dock, locked))
}

/// Height of the bar with the close button at the top of a floating window.
const CLOSE_BAR: f32 = 17.0;

fn file() -> Option<std::path::PathBuf> { crate::file_browser::config_path("layout.json") }

impl Default for Layout {
    fn default() -> Self {
        let text = file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        match from_json(&text) {
            Some((dock, locked)) => {
                let mut layout = Self { dock, locked, saved: text, sized: vec![], tab_drag: None };
                layout.restore_windows();
                layout
            }
            None                 => Self { dock: default_dock(), locked: false, saved: String::new(), sized: vec![], tab_drag: None },
        }
    }
}

impl Layout {
    pub fn reset(&mut self) { self.dock = default_dock(); }

    /// Content rectangle of a floating window, once it has been drawn.
    fn window_rect(&self, surface: usize) -> Option<bevy_egui::egui::Rect> {
        let tree = self.dock.get_surface(egui_dock::SurfaceIndex(surface))?.node_tree()?;
        let r = tree.root_node()?.rect()?;
        (r.is_finite() && r.width() > 1.0 && r.height() > 1.0).then_some(r)
    }

    /// Surfaces that are floating windows right now.
    fn windows(&self) -> Vec<usize> {
        (1..self.dock.surfaces_count())
            .filter(|i| self.dock.get_surface(egui_dock::SurfaceIndex(*i)).and_then(|s| s.node_tree()).is_some())
            .collect()
    }

    /// Put floating windows back where the layout file says they were. The
    /// file holds the rectangle of each window's content; the window itself
    /// starts above and left of it by its frame and its close bar.
    fn restore_windows(&mut self) {
        for i in self.windows() {
            let Some(r) = self.window_rect(i) else { continue };
            if let Some(state) = self.dock.get_window_state_mut(egui_dock::SurfaceIndex(i)) {
                state.set_position(r.min - bevy_egui::egui::vec2(7.0, 24.0));
                state.set_size(r.size() + bevy_egui::egui::vec2(0.0, CLOSE_BAR));
            }
            self.sized.push(i);
        }
    }

    /// Give a window that was just pulled out a workable size. Left alone it
    /// takes the size of the pane it came from, which can be taller than the
    /// screen. Call once a frame, before the panes are drawn.
    pub fn size_new_windows(&mut self) {
        let now = self.windows();
        for i in &now {
            if self.sized.contains(i) { continue; }
            if let Some(state) = self.dock.get_window_state_mut(egui_dock::SurfaceIndex(*i)) {
                state.set_size(bevy_egui::egui::vec2(440.0, 480.0));
            }
        }
        self.sized = now;
    }

    pub fn is_open(&self, pane: Pane) -> bool { self.dock.find_tab(&pane).is_some() }

    /// True when the pane sits in a floating window.
    pub fn is_floating(&self, pane: Pane) -> bool {
        self.dock.find_tab(&pane).map(|(surface, _, _)| !surface.is_main()).unwrap_or(false)
    }

    pub fn any_floating(&self) -> bool { Pane::ALL.iter().any(|p| self.is_floating(*p)) }

    /// Close a pane. It can be shown again with `show`.
    pub fn hide(&mut self, pane: Pane) {
        if let Some(at) = self.dock.find_tab(&pane) { self.dock.remove_tab(at); }
    }

    /// Show a closed pane, as a tab of the main area.
    pub fn show(&mut self, pane: Pane) {
        if self.is_open(pane) { return; }
        self.dock.main_surface_mut().push_to_focused_leaf(pane);
    }

    /// Bring a floating pane back into the main area.
    pub fn dock_pane(&mut self, pane: Pane) {
        if !self.is_floating(pane) { return; }
        self.hide(pane);
        self.show(pane);
    }

    /// Pull a docked pane out as a floating window.
    pub fn float_pane(&mut self, pane: Pane) {
        if !self.is_open(pane) || self.is_floating(pane) { return; }
        self.hide(pane);
        self.dock.add_window(vec![pane]);
    }

    pub fn toggle_float(&mut self, pane: Pane) {
        if self.is_floating(pane) { self.dock_pane(pane); } else { self.float_pane(pane); }
    }

    /// Dock a pane along a whole side of the window, next to everything
    /// else: the place the timeline has in the default layout.
    pub fn dock_to_edge(&mut self, pane: Pane, edge: Edge) {
        use egui_dock::{Node, Split};
        self.hide(pane);
        let tree = self.dock.main_surface_mut();
        if tree.root_node().map(|n| n.is_empty()).unwrap_or(true) {
            tree.push_to_first_leaf(pane);
            return;
        }
        // The fraction is the share of the left or upper part.
        let (split, fraction) = match edge {
            Edge::Left   => (Split::Left,  edge.share()),
            Edge::Top    => (Split::Above, edge.share()),
            Edge::Right  => (Split::Right, 1.0 - edge.share()),
            Edge::Bottom => (Split::Below, 1.0 - edge.share()),
        };
        tree.split(NodeIndex::root(), split, fraction, Node::leaf(pane));
    }

    pub fn dock_all(&mut self) {
        for pane in Pane::ALL { self.dock_pane(pane); }
    }

    /// Write the layout if it changed since the last write.
    pub fn save_if_changed(&mut self) {
        let Some(text) = to_json(&self.dock, self.locked) else { return };
        if text == self.saved { return; }
        if let Some(f) = file() {
            if let Some(dir) = f.parent() { let _ = std::fs::create_dir_all(dir); }
            let _ = std::fs::write(f, &text);
        }
        self.saved = text;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout { Layout { dock: default_dock(), locked: false, saved: String::new(), sized: vec![], tab_drag: None } }
    const ALL: usize = Pane::ALL.len();
    fn open(l: &Layout) -> usize { Pane::ALL.iter().filter(|p| l.is_open(**p)).count() }

    #[test]
    fn default_layout_has_every_pane_once() {
        let l = layout();
        assert!(is_valid(&l.dock));
        assert_eq!(open(&l), ALL);
        assert!(!l.any_floating());
    }

    #[test]
    fn layout_and_lock_survive_the_file() {
        let mut l = layout();
        // Stack the properties with the node graph, close the timeline, float the inspector.
        let (surface, node, _) = l.dock.find_tab(&Pane::Properties).unwrap();
        let tab = l.dock[surface][node].remove_tab(egui_dock::TabIndex(0)).unwrap();
        let (surface, node, _) = l.dock.find_tab(&Pane::NodeGraph).unwrap();
        l.dock[surface][node].append_tab(tab);
        l.hide(Pane::Timeline);
        l.hide(Pane::PrimInspector);
        l.dock.add_window(vec![Pane::PrimInspector]);
        assert!(is_valid(&l.dock));
        assert!(l.is_floating(Pane::PrimInspector) && !l.is_open(Pane::Timeline));

        let json = to_json(&l.dock, true).unwrap();
        let (back, locked) = from_json(&json).expect("layout loads");
        assert!(locked);
        assert_eq!(to_json(&back, true).unwrap(), json);
        let back = Layout { dock: back, locked, saved: json, sized: vec![], tab_drag: None };
        let (_, a, _) = back.dock.find_tab(&Pane::Properties).unwrap();
        let (_, b, _) = back.dock.find_tab(&Pane::NodeGraph).unwrap();
        assert_eq!(a, b);
        assert!(back.is_floating(Pane::PrimInspector));
        assert!(!back.is_open(Pane::Timeline));
        assert_eq!(open(&back), ALL - 1);
    }

    #[test]
    fn panes_close_reopen_and_dock_back() {
        let mut l = layout();
        l.hide(Pane::Viewport);
        l.hide(Pane::Viewport);
        assert_eq!(open(&l), ALL - 1);
        l.show(Pane::Viewport);
        l.show(Pane::Viewport);
        assert_eq!(open(&l), ALL);
        assert!(is_valid(&l.dock));

        l.hide(Pane::Properties);
        l.dock.add_window(vec![Pane::Properties]);
        l.hide(Pane::NodeGraph);
        l.dock.add_window(vec![Pane::NodeGraph]);
        assert!(l.is_floating(Pane::Properties) && l.any_floating());
        l.dock_pane(Pane::Properties);
        assert!(!l.is_floating(Pane::Properties) && l.is_floating(Pane::NodeGraph));
        l.dock_all();
        assert!(!l.any_floating());
        l.toggle_float(Pane::Timeline);
        assert!(l.is_floating(Pane::Timeline));
        l.toggle_float(Pane::Timeline);
        assert!(!l.is_floating(Pane::Timeline));
        assert_eq!(open(&l), ALL);

        // Everything closed, then one shown again.
        for p in Pane::ALL { l.hide(p); }
        assert_eq!(open(&l), 0);
        l.show(Pane::NodeGraph);
        assert!(l.is_open(Pane::NodeGraph) && !l.is_floating(Pane::NodeGraph));
        l.reset();
        assert_eq!(open(&l), ALL);
    }

    #[test]
    fn panes_dock_along_a_whole_edge() {
        use bevy_egui::egui::{pos2, Rect};
        let screen = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 800.0));
        assert_eq!(Edge::near(screen, pos2(500.0, 795.0), 30.0), Some(Edge::Bottom));
        assert_eq!(Edge::near(screen, pos2(4.0, 400.0), 30.0), Some(Edge::Left));
        assert_eq!(Edge::near(screen, pos2(990.0, 20.0), 30.0), Some(Edge::Right));
        assert_eq!(Edge::near(screen, pos2(500.0, 400.0), 30.0), None);
        assert_eq!(Edge::Bottom.strip(screen).top(), 800.0 - 800.0 * 0.16);

        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
            let mut l = layout();
            // Float the timeline, then put it back on an edge.
            l.float_pane(Pane::Timeline);
            l.dock_to_edge(Pane::Timeline, edge);
            assert!(is_valid(&l.dock) && !l.is_floating(Pane::Timeline));
            assert_eq!(open(&l), ALL);
            // It is a child of the root: it spans the whole side.
            let (node, _) = l.dock.find_main_surface_tab(&Pane::Timeline).unwrap();
            assert_eq!(node.parent(), Some(NodeIndex::root()), "{edge:?}");
            let first = matches!(edge, Edge::Left | Edge::Top);
            assert_eq!(node == NodeIndex::root().left(), first, "{edge:?}");
            // And the file still loads.
            assert!(from_json(&to_json(&l.dock, false).unwrap()).is_some());
        }
        // Onto an empty main area.
        let mut l = layout();
        for p in Pane::ALL { l.hide(p); }
        l.dock_to_edge(Pane::Viewport, Edge::Left);
        assert!(l.is_open(Pane::Viewport) && !l.is_floating(Pane::Viewport));
    }

    #[test]
    fn broken_layout_files_are_refused() {
        assert!(from_json("").is_none());
        assert!(from_json("{ nonsense").is_none());
        // The same pane twice.
        let mut dock = default_dock();
        dock.add_window(vec![Pane::Timeline]);
        assert!(!is_valid(&dock));
        assert!(from_json(&to_json(&dock, false).unwrap()).is_none());
        // A file from before the lock: the layout alone.
        let old = serde_json::to_string(&serde_json::from_str::<serde_json::Value>(&to_json(&default_dock(), false).unwrap()).unwrap()["dock"]).unwrap();
        let (dock, locked) = from_json(&old).unwrap();
        assert!(is_valid(&dock) && !locked);
    }
}
