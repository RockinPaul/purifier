use std::cmp::{Ordering, Reverse};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use purifier_core::types::{Category, FileEntry, SafetyLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::enum_variant_names,
    reason = "Tab names intentionally mirror the user-visible By Size/By Type/By Safety/By Age labels"
)]
pub enum View {
    BySize,
    ByType,
    BySafety,
    ByAge,
}

impl View {
    pub fn label(&self) -> &'static str {
        match self {
            View::BySize => "Size",
            View::ByType => "Type",
            View::BySafety => "Safety",
            View::ByAge => "Age",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    Idle,
    Scanning,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppScreen {
    DirPicker,
    Main,
}

pub struct App {
    pub screen: AppScreen,
    pub entries: Vec<FileEntry>,
    pub current_view: View,
    pub selected_index: usize,
    pub scan_status: ScanStatus,
    pub total_size: u64,
    pub total_files: u64,
    pub skipped: u64,
    pub freed_space: u64,
    pub scan_path: PathBuf,
    pub should_quit: bool,
    pub show_delete_confirm: bool,
    pub last_error: Option<String>,
    pub expanded_paths: HashSet<PathBuf>,
    pub deleted_paths: HashSet<PathBuf>,
    pub llm_enabled: bool,
    pub llm_online: bool,
    pub flat_entries: Vec<FlatEntry>,
    // Scan progress (live during scan)
    pub files_scanned: u64,
    pub bytes_found: u64,
    pub current_scan_dir: String,
    // Directory picker
    pub dir_picker_options: Vec<PathBuf>,
    pub dir_picker_selected: usize,
    pub dir_picker_custom: String,
    pub dir_picker_typing: bool,
}

#[derive(Debug, Clone)]
pub struct FlatEntry {
    pub depth: usize,
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
    pub expanded: bool,
    pub modified: Option<SystemTime>,
    pub category: Category,
    pub safety: SafetyLevel,
    pub safety_reason: String,
}

impl App {
    pub fn new(scan_path: Option<PathBuf>, llm_enabled: bool) -> Self {
        let screen = if scan_path.is_some() {
            AppScreen::Main
        } else {
            AppScreen::DirPicker
        };

        let dir_picker_options = build_dir_picker_options();

        Self {
            screen,
            entries: Vec::new(),
            current_view: View::BySize,
            selected_index: 0,
            scan_status: ScanStatus::Idle,
            total_size: 0,
            total_files: 0,
            skipped: 0,
            freed_space: 0,
            scan_path: scan_path.unwrap_or_else(|| PathBuf::from("/")),
            should_quit: false,
            show_delete_confirm: false,
            last_error: None,
            expanded_paths: HashSet::new(),
            deleted_paths: HashSet::new(),
            llm_enabled,
            llm_online: false,
            flat_entries: Vec::new(),
            files_scanned: 0,
            bytes_found: 0,
            current_scan_dir: String::new(),
            dir_picker_options,
            dir_picker_selected: 0,
            dir_picker_custom: String::new(),
            dir_picker_typing: false,
        }
    }

    pub fn start_scan_with_path(&mut self, path: PathBuf) {
        self.scan_path = path;
        self.screen = AppScreen::Main;
        self.scan_status = ScanStatus::Scanning;
        self.files_scanned = 0;
        self.bytes_found = 0;
        self.current_scan_dir.clear();
        self.expanded_paths.clear();
        self.deleted_paths.clear();
    }

    pub fn switch_view(&mut self, view: View) {
        self.current_view = view;
        self.selected_index = 0;
        self.rebuild_flat_entries();
    }

    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn move_down(&mut self) {
        let max = if self.flat_entries.is_empty() {
            0
        } else {
            self.flat_entries.len() - 1
        };
        if self.selected_index < max {
            self.selected_index += 1;
        }
    }

    pub fn toggle_expand(&mut self) {
        if let Some(flat) = self.flat_entries.get(self.selected_index) {
            if flat.is_dir {
                let path = flat.path.clone();
                if let Some(entry) = self.get_entry_mut_by_path(&path) {
                    entry.expanded = !entry.expanded;
                    if entry.expanded {
                        self.expanded_paths.insert(path.clone());
                    } else {
                        self.expanded_paths.remove(&path);
                    }
                }
                self.rebuild_flat_entries();
            }
        }
    }

    pub fn selected_entry(&self) -> Option<&FlatEntry> {
        self.flat_entries.get(self.selected_index)
    }

    pub fn remove_entry_by_path(&mut self, path: &Path) -> bool {
        fn remove_entry(entries: &mut Vec<FileEntry>, path: &Path) -> bool {
            if let Some(index) = entries.iter().position(|entry| entry.path == path) {
                entries.remove(index);
                return true;
            }

            for entry in entries {
                if remove_entry(&mut entry.children, path) {
                    return true;
                }
            }

            false
        }

        remove_entry(&mut self.entries, path)
    }

    pub fn mark_deleted(&mut self, path: &Path) {
        self.deleted_paths.insert(path.to_path_buf());
        self.expanded_paths
            .retain(|expanded| !expanded.starts_with(path));
    }

    fn get_entry_mut_by_path(&mut self, path: &Path) -> Option<&mut FileEntry> {
        fn find_entry_mut<'a>(
            entries: &'a mut [FileEntry],
            path: &Path,
        ) -> Option<&'a mut FileEntry> {
            for entry in entries {
                if entry.path == path {
                    return Some(entry);
                }

                if let Some(found) = find_entry_mut(&mut entry.children, path) {
                    return Some(found);
                }
            }

            None
        }

        find_entry_mut(&mut self.entries, path)
    }

    pub fn rebuild_flat_entries(&mut self) {
        self.flat_entries.clear();
        match self.current_view {
            View::BySize => self.flatten_by_size(),
            View::ByType => self.flatten_by_group(|e| e.category),
            View::BySafety => self.flatten_by_group(|e| e.safety),
            View::ByAge => self.flatten_by_age(),
        }
    }

    fn flatten_by_size(&mut self) {
        let mut sorted = self.entries.clone();
        sorted.sort_by_key(|entry| Reverse(entry.total_size()));

        for entry in &sorted {
            self.flatten_entry(entry, 0);
        }
    }

    fn flatten_entry(&mut self, entry: &FileEntry, depth: usize) {
        self.flat_entries.push(FlatEntry {
            depth,
            path: entry.path.clone(),
            size: entry.total_size(),
            is_dir: entry.is_dir,
            expanded: entry.expanded,
            modified: entry.modified,
            category: entry.category,
            safety: entry.safety,
            safety_reason: entry.safety_reason.clone(),
        });

        if entry.expanded {
            let mut children: Vec<&FileEntry> = entry.children.iter().collect();
            children.sort_by_key(|entry| Reverse(entry.total_size()));

            for child in children {
                self.flatten_entry(child, depth + 1);
            }
        }
    }

    fn flatten_by_group<K: Ord + std::fmt::Display, F: Fn(&FlatEntry) -> K>(&mut self, key_fn: F) {
        let mut all_flat = Vec::new();
        let sorted = {
            let mut s = self.entries.clone();
            s.sort_by_key(|entry| Reverse(entry.total_size()));
            s
        };
        for entry in &sorted {
            Self::collect_flat(entry, 0, &mut all_flat);
        }

        all_flat.sort_by(|a, b| {
            let ka = key_fn(a);
            let kb = key_fn(b);
            ka.cmp(&kb).then(b.size.cmp(&a.size))
        });

        self.flat_entries = all_flat;
    }

    fn flatten_by_age(&mut self) {
        let mut all_flat = Vec::new();
        for entry in &self.entries {
            Self::collect_flat(entry, 0, &mut all_flat);
        }

        all_flat.sort_by(|a, b| match (a.modified, b.modified) {
            (Some(a_modified), Some(b_modified)) => {
                a_modified.cmp(&b_modified).then(b.size.cmp(&a.size))
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => b.size.cmp(&a.size),
        });

        self.flat_entries = all_flat;
    }

    fn collect_flat(entry: &FileEntry, depth: usize, out: &mut Vec<FlatEntry>) {
        out.push(FlatEntry {
            depth,
            path: entry.path.clone(),
            size: entry.total_size(),
            is_dir: entry.is_dir,
            expanded: entry.expanded,
            modified: entry.modified,
            category: entry.category,
            safety: entry.safety,
            safety_reason: entry.safety_reason.clone(),
        });

        for child in &entry.children {
            Self::collect_flat(child, depth + 1, out);
        }
    }
}

fn build_dir_picker_options() -> Vec<PathBuf> {
    let mut options = vec![PathBuf::from("/")];

    if let Some(home) = dirs::home_dir() {
        options.push(home.clone());

        let candidates = [
            home.join("Downloads"),
            home.join("Library"),
            home.join("Documents"),
            home.join("Projects"),
            home.join("Developer"),
            home.join("Desktop"),
        ];

        for path in candidates {
            if path.exists() {
                options.push(path);
            }
        }
    }

    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn dir(path: &str, size: u64) -> FileEntry {
        FileEntry::new(PathBuf::from(path), size, true, None)
    }

    fn file(path: &str, size: u64, modified: Option<SystemTime>) -> FileEntry {
        FileEntry::new(PathBuf::from(path), size, false, modified)
    }

    #[test]
    fn toggle_expand_should_target_selected_path_when_entries_are_size_sorted() {
        let mut app = App::new(Some(PathBuf::from("/")), false);
        app.entries = vec![dir("/small", 10), dir("/large", 20)];
        app.rebuild_flat_entries();

        assert_eq!(
            app.selected_entry().map(|entry| entry.path.as_path()),
            Some(PathBuf::from("/large").as_path())
        );

        app.toggle_expand();

        assert!(!app.entries[0].expanded, "small should stay collapsed");
        assert!(app.entries[1].expanded, "large should expand");
    }

    #[test]
    fn remove_entry_by_path_should_remove_selected_path_when_entries_are_size_sorted() {
        let mut app = App::new(Some(PathBuf::from("/")), false);
        app.entries = vec![dir("/small", 10), dir("/large", 20)];
        app.rebuild_flat_entries();

        let selected_path = app
            .selected_entry()
            .map(|entry| entry.path.clone())
            .expect("selected entry should exist");

        assert!(app.remove_entry_by_path(&selected_path));
        assert_eq!(app.entries.len(), 1, "one entry should remain");
        assert_eq!(app.entries[0].path, PathBuf::from("/small"));
    }

    #[test]
    fn age_view_should_sort_oldest_first_and_put_missing_timestamps_last() {
        let older = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let newer = SystemTime::UNIX_EPOCH + Duration::from_secs(20);
        let mut app = App::new(Some(PathBuf::from("/")), false);
        app.entries = vec![
            file("/none", 5, None),
            file("/newer", 5, Some(newer)),
            file("/older", 5, Some(older)),
        ];

        app.switch_view(View::ByAge);

        let paths: Vec<PathBuf> = app
            .flat_entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/older"),
                PathBuf::from("/newer"),
                PathBuf::from("/none"),
            ]
        );
    }

    #[test]
    fn age_view_should_use_size_as_tiebreaker_for_matching_timestamps() {
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let mut app = App::new(Some(PathBuf::from("/")), false);
        app.entries = vec![
            file("/small", 10, Some(modified)),
            file("/large", 20, Some(modified)),
        ];

        app.switch_view(View::ByAge);

        let paths: Vec<PathBuf> = app
            .flat_entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        assert_eq!(
            paths,
            vec![PathBuf::from("/large"), PathBuf::from("/small")]
        );
    }
}
