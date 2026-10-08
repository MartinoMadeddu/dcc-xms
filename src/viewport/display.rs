//! How the viewport draws meshes.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    /// Polygon edges only, with those behind a surface hidden.
    HiddenLine,
    /// Every polygon edge, seen through.
    Wireframe,
    Shaded,
    /// Shaded, with the materials and textures of USD primitives.
    Textured,
}

impl DisplayMode {
    pub const ALL: [DisplayMode; 4] = [DisplayMode::Wireframe, DisplayMode::HiddenLine, DisplayMode::Shaded, DisplayMode::Textured];
    pub fn label(self) -> &'static str {
        match self {
            DisplayMode::HiddenLine => "Hidden Line Removal",
            DisplayMode::Wireframe  => "Wireframe",
            DisplayMode::Shaded     => "Shaded",
            DisplayMode::Textured   => "Textured",
        }
    }
    fn code(self) -> u8 { Self::ALL.iter().position(|m| *m == self).unwrap_or(3) as u8 }
    fn from_code(c: u8) -> Self { Self::ALL.get(c as usize).copied().unwrap_or(DisplayMode::Textured) }
    fn key(self) -> &'static str {
        match self { DisplayMode::HiddenLine => "hidden_line", DisplayMode::Wireframe => "wireframe", DisplayMode::Shaded => "shaded", DisplayMode::Textured => "textured" }
    }
}

static MODE: AtomicU8 = AtomicU8::new(3);
static LOADED: std::sync::Once = std::sync::Once::new();

fn file() -> Option<std::path::PathBuf> { crate::file_browser::config_path("display.txt") }

/// The current mode. The remembered one is read the first time.
pub fn mode() -> DisplayMode {
    LOADED.call_once(|| {
        let saved = file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        if let Some(m) = DisplayMode::ALL.iter().find(|m| m.key() == saved.trim()) { MODE.store(m.code(), Ordering::Relaxed); }
    });
    DisplayMode::from_code(MODE.load(Ordering::Relaxed))
}

/// Switch mode, remember it, and have the viewport redraw.
pub fn set_mode(m: DisplayMode) {
    if mode() == m { return; }
    MODE.store(m.code(), Ordering::Relaxed);
    super::textures::touch();
    if let Some(f) = file() {
        if let Some(dir) = f.parent() { let _ = std::fs::create_dir_all(dir); }
        let _ = std::fs::write(f, m.key());
    }
}

/// The edges to draw for a mesh, as pairs of vertex indices: the sides of
/// its polygons, each once. Triangles inside a polygon add nothing.
pub fn wire_edges(mesh: &crate::types::MeshData) -> Vec<u32> {
    let mut keys: Vec<u64> = Vec::with_capacity(mesh.indices.len());
    let mut add = |a: u32, b: u32| { let (lo, hi) = if a < b { (a, b) } else { (b, a) }; if lo != hi { keys.push(((lo as u64) << 32) | hi as u64); } };
    if mesh.polys.is_empty() {
        for t in mesh.indices.chunks_exact(3) { add(t[0], t[1]); add(t[1], t[2]); add(t[2], t[0]); }
    } else {
        for p in &mesh.polys { for k in 0..p.len() { add(p[k], p[(k + 1) % p.len()]); } }
    }
    keys.sort_unstable();
    keys.dedup();
    let mut out = Vec::with_capacity(keys.len() * 2);
    for k in keys { out.push((k >> 32) as u32); out.push(k as u32); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MeshData;

    #[test]
    fn polygon_sides_are_drawn_once_and_diagonals_not_at_all() {
        // Two quads sharing a side, each triangulated in two.
        let mut m = MeshData::from_triangles(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0]],
            vec![0, 1, 2, 0, 2, 3, 1, 4, 5, 1, 5, 2]);
        m.polys = vec![vec![0, 1, 2, 3], vec![1, 4, 5, 2]];
        let e = wire_edges(&m);
        // Seven sides: four and four, less the shared one.
        assert_eq!(e.len(), 7 * 2);
        let pairs: Vec<(u32, u32)> = e.chunks_exact(2).map(|p| (p[0], p[1])).collect();
        assert!(!pairs.contains(&(0, 2)) && !pairs.contains(&(1, 5)), "no diagonals");
        assert_eq!(pairs.iter().filter(|p| **p == (1, 2)).count(), 1);
        // Without polygons, every triangle side counts.
        m.polys.clear();
        assert_eq!(wire_edges(&m).len(), 9 * 2);
    }

    #[test]
    fn modes_have_distinct_codes_and_names() {
        for m in DisplayMode::ALL { assert_eq!(DisplayMode::from_code(m.code()), m); }
        assert_eq!(DisplayMode::HiddenLine.label(), "Hidden Line Removal");
    }
}
