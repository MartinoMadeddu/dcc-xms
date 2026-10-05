//! In-app file browser.
//!
//! Opens where the user last navigated, in this session or a previous one.
//! The location is kept in a small text file in the user's config folder.

use std::path::{Path, PathBuf};

use bevy::prelude::Resource;
use bevy_egui::egui;

use crate::types::NodeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseMode {
    /// Pick an existing file.
    File,
    /// Pick a folder.
    Folder,
    /// Pick a folder and type a file name.
    Save,
}

/// Who asked for the path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseTarget {
    /// The path parameter of a node.
    Node(NodeId),
    OpenGraph,
    SaveGraph,
}

struct Request {
    target:    BrowseTarget,
    mode:      BrowseMode,
    title:     String,
    /// Lower-case extensions shown in File and Save mode. Empty shows all.
    exts:      Vec<String>,
    selected:  Option<PathBuf>,
    file_name: String,
    path_edit: String,
    all_files: bool,
}

#[derive(Resource)]
pub struct FileBrowser {
    dir:     PathBuf,
    request: Option<Request>,
    result:  Option<(BrowseTarget, PathBuf)>,
}

impl Default for FileBrowser {
    fn default() -> Self {
        Self { dir: load_last_dir(), request: None, result: None }
    }
}

fn config_file() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("xms").join("last_dir.txt"))
}

fn home_dir() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn load_last_dir() -> PathBuf {
    config_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| p.is_dir())
        .unwrap_or_else(home_dir)
}

fn save_last_dir(dir: &Path) {
    let Some(file) = config_file() else { return };
    if let Some(parent) = file.parent() { let _ = std::fs::create_dir_all(parent); }
    let _ = std::fs::write(file, dir.to_string_lossy().as_bytes());
}

impl FileBrowser {
    pub fn is_open(&self) -> bool { self.request.is_some() }

    /// Open the browser in the last visited folder.
    pub fn open(&mut self, target: BrowseTarget, mode: BrowseMode, title: &str, exts: &[&str], file_name: &str) {
        if !self.dir.is_dir() { self.dir = home_dir(); }
        self.request = Some(Request {
            target,
            mode,
            title: title.to_string(),
            exts: exts.iter().map(|e| e.to_lowercase()).collect(),
            selected: None,
            file_name: file_name.to_string(),
            path_edit: self.dir.to_string_lossy().to_string(),
            all_files: false,
        });
    }

    /// The chosen path, once. Returned the frame after the user confirms.
    pub fn take_result(&mut self) -> Option<(BrowseTarget, PathBuf)> { self.result.take() }

    fn go(&mut self, dir: PathBuf) {
        if !dir.is_dir() { return; }
        self.dir = dir;
        save_last_dir(&self.dir);
        if let Some(r) = &mut self.request {
            r.selected  = None;
            r.path_edit = self.dir.to_string_lossy().to_string();
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let Some(req) = self.request.as_mut() else { return };

        // Folder contents: folders first, then files that match the filter.
        let mut dirs:  Vec<PathBuf> = vec![];
        let mut files: Vec<PathBuf> = vec![];
        let mut read_error = None;
        match std::fs::read_dir(&self.dir) {
            Ok(rd) => for e in rd.filter_map(|e| e.ok()) {
                let p = e.path();
                let hidden = p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false);
                if hidden { continue; }
                if p.is_dir() {
                    dirs.push(p);
                } else if req.mode != BrowseMode::Folder {
                    let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                    if req.all_files || req.exts.is_empty() || req.exts.contains(&ext) { files.push(p); }
                }
            },
            Err(e) => read_error = Some(e.to_string()),
        }
        let key = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        dirs.sort_by_key(key);
        files.sort_by_key(key);

        let mut go_to:   Option<PathBuf> = None;
        let mut confirm: Option<PathBuf> = None;
        let mut cancel = false;
        let mut open = true;

        egui::Window::new(req.title.clone())
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 460.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                // ── Location bar ─────────────────────────────────────────────
                ui.horizontal(|ui| {
                    if ui.button("⬆ Up").clicked() {
                        if let Some(p) = self.dir.parent() { go_to = Some(p.to_path_buf()); }
                    }
                    if ui.button("🏠 Home").clicked() { go_to = Some(home_dir()); }
                    let edit = ui.add(egui::TextEdit::singleline(&mut req.path_edit).desired_width(f32::INFINITY));
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        go_to = Some(PathBuf::from(req.path_edit.trim()));
                    }
                });
                if cfg!(windows) {
                    ui.horizontal_wrapped(|ui| {
                        for letter in b'A'..=b'Z' {
                            let drive = PathBuf::from(format!("{}:\\", letter as char));
                            if drive.is_dir() && ui.small_button(format!("{}:", letter as char)).clicked() {
                                go_to = Some(drive);
                            }
                        }
                    });
                }
                ui.separator();

                // ── Entries ──────────────────────────────────────────────────
                let footer = if req.mode == BrowseMode::Save { 64.0 } else { 36.0 };
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height((ui.available_height() - footer).max(80.0))
                    .show(ui, |ui| {
                        if let Some(e) = &read_error {
                            ui.colored_label(egui::Color32::from_rgb(220, 140, 140), e);
                        }
                        for d in &dirs {
                            let name = d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                            let resp = ui.selectable_label(false, format!("📁 {name}"));
                            if resp.double_clicked() || resp.clicked() { go_to = Some(d.clone()); }
                        }
                        for f in &files {
                            let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                            let sel  = req.selected.as_ref() == Some(f);
                            let resp = ui.selectable_label(sel, format!("📄 {name}"));
                            if resp.clicked() {
                                req.selected  = Some(f.clone());
                                req.file_name = name.clone();
                            }
                            if resp.double_clicked() { confirm = Some(f.clone()); }
                        }
                        if dirs.is_empty() && files.is_empty() && read_error.is_none() {
                            ui.weak("Nothing to show here.");
                        }
                    });
                ui.separator();

                // ── Confirm ──────────────────────────────────────────────────
                if req.mode == BrowseMode::Save {
                    ui.horizontal(|ui| {
                        ui.label("File name:");
                        ui.add(egui::TextEdit::singleline(&mut req.file_name).desired_width(f32::INFINITY));
                    });
                }
                ui.horizontal(|ui| {
                    match req.mode {
                        BrowseMode::File => {
                            if ui.add_enabled(req.selected.is_some(), egui::Button::new("Open")).clicked() {
                                confirm = req.selected.clone();
                            }
                        }
                        BrowseMode::Folder => {
                            if ui.button("Use this folder").clicked() { confirm = Some(self.dir.clone()); }
                        }
                        BrowseMode::Save => {
                            let name = req.file_name.trim().to_string();
                            if ui.add_enabled(!name.is_empty(), egui::Button::new("Save")).clicked() {
                                let mut p = self.dir.join(&name);
                                if p.extension().is_none() {
                                    if let Some(ext) = req.exts.first() { p.set_extension(ext); }
                                }
                                confirm = Some(p);
                            }
                        }
                    }
                    if ui.button("Cancel").clicked() { cancel = true; }
                    if req.mode != BrowseMode::Folder && !req.exts.is_empty() {
                        ui.checkbox(&mut req.all_files, "All files");
                        ui.weak(format!("showing .{}", req.exts.join(" .")));
                    }
                });
            });

        if let Some(dir) = go_to { self.go(dir); }
        if let Some(path) = confirm {
            let target = self.request.as_ref().map(|r| r.target);
            if let Some(target) = target { self.result = Some((target, path)); }
            save_last_dir(&self.dir);
            self.request = None;
        } else if cancel || !open {
            self.request = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopens_in_last_visited_folder() {
        let base = std::env::temp_dir().join("xms_browser_test");
        let _ = std::fs::remove_dir_all(&base);
        let visited = base.join("takes").join("day01");
        std::fs::create_dir_all(&visited).unwrap();
        // Keep the config file inside the test folder.
        std::env::set_var(if cfg!(windows) { "APPDATA" } else { "XDG_CONFIG_HOME" }, base.join("cfg"));

        let mut b = FileBrowser::default();
        b.open(BrowseTarget::OpenGraph, BrowseMode::File, "Open", &["fbx"], "");
        b.go(visited.clone());
        assert_eq!(b.dir, visited);
        // Not a folder: ignored.
        b.go(base.join("missing"));
        assert_eq!(b.dir, visited);

        // A new session starts where the last one left off.
        let again = FileBrowser::default();
        assert_eq!(again.dir, visited);

        // If that folder is gone, fall back to the home folder.
        std::fs::remove_dir_all(&visited).unwrap();
        assert_ne!(FileBrowser::default().dir, visited);
    }
}
