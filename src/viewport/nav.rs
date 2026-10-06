//! Viewport navigation styles: which keys and mouse buttons orbit, pan and
//! zoom, modelled on other packages. Plain data and functions, no systems.

use bevy::math::Vec2;
use bevy::prelude::Resource;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NavStyle { #[default] Maya, Houdini, Xsi, Blender, Max, Modo, Unreal }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavAction { Orbit, Pan, Zoom }

/// Keys and buttons held right now. `alt` includes Cmd / Super.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub alt: bool, pub shift: bool, pub ctrl: bool, pub space: bool, pub s: bool,
    pub lmb: bool, pub mmb: bool, pub rmb: bool,
}

impl NavStyle {
    pub const ALL: [NavStyle; 7] = [
        NavStyle::Maya, NavStyle::Houdini, NavStyle::Xsi, NavStyle::Blender,
        NavStyle::Max, NavStyle::Modo, NavStyle::Unreal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            NavStyle::Maya => "Maya", NavStyle::Houdini => "Houdini", NavStyle::Xsi => "XSI",
            NavStyle::Blender => "Blender", NavStyle::Max => "Max", NavStyle::Modo => "Modo",
            NavStyle::Unreal => "Unreal",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|n| n.label().eq_ignore_ascii_case(s.trim()))
    }

    /// What the held keys and buttons do to the camera.
    pub fn action(self, h: Held) -> Option<NavAction> {
        use NavAction::*;
        // Alt (or another key) with the three buttons, as in Maya.
        let maya = |key: bool| -> Option<NavAction> {
            if !key { return None; }
            if h.lmb && h.mmb { Some(Zoom) }
            else if h.lmb { Some(Orbit) }
            else if h.mmb { Some(Pan) }
            else if h.rmb { Some(Zoom) }
            else { None }
        };
        match self {
            NavStyle::Maya | NavStyle::Unreal => maya(h.alt),
            NavStyle::Houdini => maya(h.space || h.alt),
            NavStyle::Xsi => {
                if !h.s { None }
                else if h.rmb { Some(Orbit) }
                else if h.lmb { Some(Pan) }
                else if h.mmb { Some(Zoom) }
                else { None }
            }
            NavStyle::Blender => {
                if !h.mmb { None }
                else if h.shift { Some(Pan) }
                else if h.ctrl { Some(Zoom) }
                else { Some(Orbit) }
            }
            NavStyle::Max => {
                if !h.mmb { None }
                else if h.ctrl && h.alt { Some(Zoom) }
                else if h.alt { Some(Orbit) }
                else { Some(Pan) }
            }
            NavStyle::Modo => {
                if !(h.alt && h.lmb) { None }
                else if h.shift { Some(Pan) }
                else if h.ctrl { Some(Zoom) }
                else { Some(Orbit) }
            }
        }
    }

    /// True while a key is held that hands the left mouse button to the
    /// camera, so selection tools stay out of the way.
    pub fn claims_left_button(self, h: Held) -> bool {
        match self {
            NavStyle::Maya | NavStyle::Unreal | NavStyle::Modo => h.alt,
            NavStyle::Houdini => h.alt || h.space,
            NavStyle::Xsi => h.s,
            // Blender and Max navigate on the middle button only. Alt is
            // still kept clear, as it was before styles existed.
            NavStyle::Blender | NavStyle::Max => h.alt,
        }
    }

    /// Zoom from a drag in pixels: positive moves in. Horizontal styles
    /// zoom in to the right, vertical ones upwards.
    pub fn zoom(self, delta: Vec2) -> f32 {
        match self {
            NavStyle::Maya | NavStyle::Houdini | NavStyle::Unreal | NavStyle::Modo => delta.x,
            NavStyle::Xsi | NavStyle::Blender | NavStyle::Max => -delta.y,
        }
    }

    /// 1 when a pan drag pulls the scene along with the cursor, -1 when it
    /// moves the camera with the cursor instead.
    pub fn pan_sign(self) -> f32 {
        if self == NavStyle::Unreal { -1.0 } else { 1.0 }
    }

    /// Lines for the help box in the viewport.
    pub fn help(self) -> [&'static str; 3] {
        match self {
            NavStyle::Maya    => ["Alt + LMB: orbit", "Alt + MMB: pan", "Alt + RMB: zoom (left / right)"],
            NavStyle::Unreal  => ["Alt + LMB: orbit", "Alt + MMB: pan (reversed)", "Alt + RMB: zoom (left / right)"],
            NavStyle::Houdini => ["Space + LMB: orbit", "Space + MMB: pan", "Space + RMB: zoom (left / right)"],
            NavStyle::Xsi     => ["S + RMB: orbit", "S + LMB: pan", "S + MMB: zoom (up / down)"],
            NavStyle::Blender => ["MMB: orbit", "Shift + MMB: pan", "Ctrl + MMB: zoom (up / down)"],
            NavStyle::Max     => ["Alt + MMB: orbit", "MMB: pan", "Ctrl + Alt + MMB: zoom (up / down)"],
            NavStyle::Modo    => ["Alt + LMB: orbit", "Alt + Shift + LMB: pan", "Alt + Ctrl + LMB: zoom (left / right)"],
        }
    }
}

/// The chosen style. Kept between sessions in the config folder.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct NavSettings {
    pub style:       NavStyle,
    /// Swap the direction of the zoom drag.
    pub invert_zoom: bool,
}

impl Default for NavSettings {
    fn default() -> Self {
        crate::file_browser::config_path("navigation.txt")
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|s| Self::parse(&s))
            .unwrap_or(Self { style: NavStyle::Maya, invert_zoom: false })
    }
}

impl NavSettings {
    pub fn parse(text: &str) -> Self {
        let mut lines = text.lines();
        Self {
            style: lines.next().and_then(NavStyle::from_label).unwrap_or_default(),
            invert_zoom: lines.next().map(|l| l.trim() == "invert").unwrap_or(false),
        }
    }

    pub fn to_text(self) -> String {
        format!("{}\n{}\n", self.style.label(), if self.invert_zoom { "invert" } else { "normal" })
    }

    pub fn save(self) {
        let Some(file) = crate::file_browser::config_path("navigation.txt") else { return };
        if let Some(parent) = file.parent() { let _ = std::fs::create_dir_all(parent); }
        let _ = std::fs::write(file, self.to_text());
    }

    pub fn zoom(self, delta: Vec2) -> f32 {
        self.style.zoom(delta) * if self.invert_zoom { -1.0 } else { 1.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use NavAction::*;

    fn h(keys: &str) -> Held {
        let mut out = Held::default();
        for k in keys.split('+').map(|k| k.trim()) {
            match k {
                "alt" => out.alt = true, "shift" => out.shift = true, "ctrl" => out.ctrl = true,
                "space" => out.space = true, "s" => out.s = true,
                "lmb" => out.lmb = true, "mmb" => out.mmb = true, "rmb" => out.rmb = true,
                "" => {}
                other => panic!("unknown key {other}"),
            }
        }
        out
    }

    #[test]
    fn each_style_maps_its_own_combinations() {
        let cases: &[(NavStyle, &str, Option<NavAction>)] = &[
            (NavStyle::Maya, "alt+lmb", Some(Orbit)), (NavStyle::Maya, "alt+mmb", Some(Pan)),
            (NavStyle::Maya, "alt+rmb", Some(Zoom)), (NavStyle::Maya, "alt+lmb+mmb", Some(Zoom)),
            (NavStyle::Maya, "lmb", None), (NavStyle::Maya, "mmb", None), (NavStyle::Maya, "space+lmb", None),
            (NavStyle::Houdini, "space+lmb", Some(Orbit)), (NavStyle::Houdini, "space+mmb", Some(Pan)),
            (NavStyle::Houdini, "space+rmb", Some(Zoom)), (NavStyle::Houdini, "alt+lmb", Some(Orbit)),
            (NavStyle::Houdini, "lmb", None),
            (NavStyle::Xsi, "s+rmb", Some(Orbit)), (NavStyle::Xsi, "s+lmb", Some(Pan)),
            (NavStyle::Xsi, "s+mmb", Some(Zoom)), (NavStyle::Xsi, "alt+lmb", None), (NavStyle::Xsi, "rmb", None),
            (NavStyle::Blender, "mmb", Some(Orbit)), (NavStyle::Blender, "shift+mmb", Some(Pan)),
            (NavStyle::Blender, "ctrl+mmb", Some(Zoom)), (NavStyle::Blender, "alt+lmb", None),
            (NavStyle::Max, "mmb", Some(Pan)), (NavStyle::Max, "alt+mmb", Some(Orbit)),
            (NavStyle::Max, "ctrl+alt+mmb", Some(Zoom)), (NavStyle::Max, "alt+lmb", None),
            (NavStyle::Modo, "alt+lmb", Some(Orbit)), (NavStyle::Modo, "alt+shift+lmb", Some(Pan)),
            (NavStyle::Modo, "alt+ctrl+lmb", Some(Zoom)), (NavStyle::Modo, "alt+mmb", None),
            (NavStyle::Unreal, "alt+lmb", Some(Orbit)), (NavStyle::Unreal, "alt+mmb", Some(Pan)),
            (NavStyle::Unreal, "alt+rmb", Some(Zoom)),
        ];
        for (style, keys, want) in cases {
            assert_eq!(style.action(h(keys)), *want, "{} with {keys}", style.label());
        }
        // No button, no action, in any style.
        for style in NavStyle::ALL {
            assert_eq!(style.action(h("alt+shift+ctrl+space+s")), None);
        }
    }

    #[test]
    fn zoom_and_pan_directions() {
        let right = Vec2::new(10.0, 0.0);
        let up = Vec2::new(0.0, -10.0);   // screen y grows downwards
        for style in [NavStyle::Maya, NavStyle::Houdini, NavStyle::Unreal, NavStyle::Modo] {
            assert!(style.zoom(right) > 0.0 && style.zoom(-right) < 0.0);
            assert_eq!(style.zoom(up), 0.0, "{}", style.label());
        }
        for style in [NavStyle::Blender, NavStyle::Max, NavStyle::Xsi] {
            assert!(style.zoom(up) > 0.0 && style.zoom(-up) < 0.0);
            assert_eq!(style.zoom(right), 0.0);
        }
        for style in NavStyle::ALL {
            assert_eq!(style.pan_sign(), if style == NavStyle::Unreal { -1.0 } else { 1.0 });
        }
        let inv = NavSettings { style: NavStyle::Maya, invert_zoom: true };
        assert!(inv.zoom(right) < 0.0);
    }

    #[test]
    fn selection_keeps_clear_of_the_navigation_key() {
        assert!(NavStyle::Maya.claims_left_button(h("alt")));
        assert!(!NavStyle::Maya.claims_left_button(h("space")));
        assert!(NavStyle::Houdini.claims_left_button(h("space")));
        assert!(NavStyle::Xsi.claims_left_button(h("s")));
        assert!(!NavStyle::Xsi.claims_left_button(h("ctrl")));
        assert!(!NavStyle::Blender.claims_left_button(h("shift")));
    }

    #[test]
    fn settings_round_trip_as_text() {
        for style in NavStyle::ALL {
            for invert_zoom in [false, true] {
                let s = NavSettings { style, invert_zoom };
                assert_eq!(NavSettings::parse(&s.to_text()), s);
            }
        }
        assert_eq!(NavSettings::parse("nonsense"), NavSettings { style: NavStyle::Maya, invert_zoom: false });
        assert_eq!(NavSettings::parse(""), NavSettings { style: NavStyle::Maya, invert_zoom: false });
    }
}
