//! Dockable panes: which panes exist, where they sit, and the layout file.
//!
//! Every pane is a tab of one dock area. Tabs can be dragged to another
//! place, stacked, or pulled out as floating windows. The layout is kept in
//! the config folder and restored at the next start.

use bevy::prelude::Resource;
use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pane { Viewport, NodeGraph, SceneExplorer, OperatorStack, Properties, PrimInspector, Timeline }

impl Pane {
    pub const ALL: [Pane; 7] = [
        Pane::Viewport, Pane::NodeGraph, Pane::SceneExplorer, Pane::OperatorStack,
        Pane::Properties, Pane::PrimInspector, Pane::Timeline,
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
        }
    }
}

#[derive(Resource)]
pub struct Layout {
    pub dock: DockState<Pane>,
    /// The layout as last written to disk.
    saved:    String,
}

/// Viewport on the left with the inspector under it, then the node graph,
/// the scene and stack column, and the properties. Timeline along the bottom.
pub fn default_dock() -> DockState<Pane> {
    let mut dock = DockState::new(vec![Pane::Viewport]);
    let s = dock.main_surface_mut();
    let [top, _timeline] = s.split_below(NodeIndex::root(), 0.9, vec![Pane::Timeline]);
    let [viewport, rest] = s.split_right(top, 0.37, vec![Pane::NodeGraph]);
    let [_graph, rest]   = s.split_right(rest, 0.57, vec![Pane::SceneExplorer]);
    let [scene, _props]  = s.split_right(rest, 0.5, vec![Pane::Properties]);
    s.split_below(scene, 0.5, vec![Pane::OperatorStack]);
    s.split_below(viewport, 0.72, vec![Pane::PrimInspector]);
    dock
}

/// A layout is usable when every pane is in it exactly once.
pub fn is_complete(dock: &DockState<Pane>) -> bool {
    let tabs: Vec<Pane> = dock.iter_all_tabs().map(|(_, t)| *t).collect();
    tabs.len() == Pane::ALL.len() && Pane::ALL.iter().all(|p| tabs.contains(p))
}

/// The layout as text. Rectangles of panes that have not been laid out yet
/// are infinite, which JSON cannot hold: they are written as zero. They are
/// worked out again on the first frame anyway.
pub fn to_json(dock: &DockState<Pane>) -> Option<String> {
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
    serde_json::to_string(&value).ok()
}

pub fn from_json(json: &str) -> Option<DockState<Pane>> {
    serde_json::from_str::<DockState<Pane>>(json).ok().filter(is_complete)
}

fn file() -> Option<std::path::PathBuf> { crate::file_browser::config_path("layout.json") }

impl Default for Layout {
    fn default() -> Self {
        let text = file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        match from_json(&text) {
            Some(dock) => Self { dock, saved: text },
            None       => Self { dock: default_dock(), saved: String::new() },
        }
    }
}

impl Layout {
    pub fn reset(&mut self) { self.dock = default_dock(); }

    /// Write the layout if it changed since the last write.
    pub fn save_if_changed(&mut self) {
        let Some(text) = to_json(&self.dock) else { return };
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

    #[test]
    fn default_layout_has_every_pane_once() {
        assert!(is_complete(&default_dock()));
    }

    #[test]
    fn layout_survives_the_file() {
        let mut dock = default_dock();
        // Move things about: stack the properties with the node graph.
        let (surface, node, _) = dock.find_tab(&Pane::Properties).unwrap();
        let tab = dock[surface][node].remove_tab(egui_dock::TabIndex(0)).unwrap();
        let (surface, node, _) = dock.find_tab(&Pane::NodeGraph).unwrap();
        dock[surface][node].append_tab(tab);
        assert!(is_complete(&dock));

        let json = to_json(&dock).unwrap();
        let back = from_json(&json).expect("layout loads");
        assert_eq!(to_json(&back).unwrap(), json);
        let (_, a, _) = back.find_tab(&Pane::Properties).unwrap();
        let (_, b, _) = back.find_tab(&Pane::NodeGraph).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn broken_layout_files_are_refused() {
        assert!(from_json("").is_none());
        assert!(from_json("{ nonsense").is_none());
        // A pane missing: the default is used instead.
        let mut dock = default_dock();
        let (surface, node, _) = dock.find_tab(&Pane::Timeline).unwrap();
        dock[surface][node].remove_tab(egui_dock::TabIndex(0));
        assert!(!is_complete(&dock));
        assert!(from_json(&to_json(&dock).unwrap()).is_none());
    }
}
