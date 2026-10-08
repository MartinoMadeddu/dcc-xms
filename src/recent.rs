//! Graphs opened or saved lately, newest first, kept in `recent.json` in the
//! config folder.

use std::path::{Path, PathBuf};

use bevy::prelude::Resource;

/// How many files the list keeps.
pub const KEEP: usize = 10;

#[derive(Resource)]
pub struct Recent {
    pub files: Vec<PathBuf>,
    file:      Option<PathBuf>,
}

impl Default for Recent {
    fn default() -> Self { Self::at(crate::file_browser::config_path("recent.json")) }
}

impl Recent {
    /// The list kept in `file` (none: kept in memory only).
    pub fn at(file: Option<PathBuf>) -> Self {
        let files = file.as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
            .unwrap_or_default()
            .into_iter().map(PathBuf::from).take(KEEP).collect();
        Self { files, file }
    }

    /// Put a file at the top. It is kept by its full path.
    pub fn add(&mut self, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.files.retain(|f| *f != path);
        self.files.insert(0, path);
        self.files.truncate(KEEP);
        self.save();
    }

    pub fn remove(&mut self, path: &Path) {
        self.files.retain(|f| f != path);
        self.save();
    }

    pub fn clear(&mut self) {
        self.files.clear();
        self.save();
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() { let _ = std::fs::create_dir_all(dir); }
        let list: Vec<String> = self.files.iter().map(|f| f.to_string_lossy().to_string()).collect();
        let _ = std::fs::write(file, serde_json::to_string_pretty(&list).unwrap_or_default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_no_repeats_at_most_ten_and_kept_between_starts() {
        let file = std::env::temp_dir().join("xms_recent_test.json");
        let _ = std::fs::remove_file(&file);
        let mut r = Recent::at(Some(file.clone()));
        for k in 0..12 { r.add(&std::env::temp_dir().join(format!("g{k}.json"))); }
        r.add(&std::env::temp_dir().join("g5.json"));
        assert_eq!(r.files.len(), KEEP);
        assert!(r.files[0].ends_with("g5.json"));
        assert_eq!(r.files.iter().filter(|f| f.ends_with("g5.json")).count(), 1);
        assert!(r.files[1].ends_with("g11.json"));
        let again = Recent::at(Some(file.clone()));
        assert_eq!(again.files, r.files);
        r.clear();
        assert!(Recent::at(Some(file)).files.is_empty());
    }
}
