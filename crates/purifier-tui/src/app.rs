use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use purifier_core::provider::{default_provider_settings, ProviderKind};
use purifier_core::size::SizeMode;
use purifier_core::types::{Category, FileEntry, SafetyLevel};
use purifier_core::DeleteOutcome;

use crate::columns::{find_children, find_entry, sorted_children_cached, ColumnStack};
use crate::config::AppConfig;
use crate::marks::MarkSet;
use crate::ui::preview_pane::{aggregate_by_age, aggregate_by_category};

/// Cached preview analytics for the currently selected directory.
#[derive(Debug, Clone)]
pub struct PreviewAnalytics {
    pub by_category: Vec<(Category, u64)>,
    pub by_age: Vec<(&'static str, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    Idle,
    Scanning,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppScreen {
    Onboarding,
    DirPicker,
    Main,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmStatus {
    Disabled,
    NeedsSetup,
    Connecting(ProviderKind),
    Ready(ProviderKind),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsDraft {
    pub provider: ProviderKind,
    pub api_key: String,
    pub api_key_edited: bool,
    pub api_key_editing: bool,
    pub model: String,
    pub base_url: String,
    pub llm_enabled: bool,
    pub size_mode: SizeMode,
    pub selected_scan_profile: Option<String>,
}

/// What the preview (right) pane currently shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewMode {
    /// Type/age/safety analytics for the selected entry.
    Analytics,
    /// Quick-delete confirmation for one path.
    DeleteConfirm(PathBuf),
    /// Batch-delete review of all marked items.
    BatchReview,
    /// Settings form rendered in the preview pane.
    Settings(SettingsDraft),
    /// Onboarding form (only used on AppScreen::Onboarding).
    Onboarding(SettingsDraft),
}

pub struct App {
    pub screen: AppScreen,
    pub entries: Vec<FileEntry>,
    pub columns: ColumnStack,
    pub marks: MarkSet,
    pub preview_mode: PreviewMode,
    pub scan_status: ScanStatus,
    pub total_size: u64,
    pub total_logical_size: u64,
    pub total_physical_size: u64,
    pub total_files: u64,
    pub skipped: u64,
    pub delete_stats: DeleteOutcome,
    pub scan_path: PathBuf,
    pub should_quit: bool,
    pub preferences: AppConfig,
    pub settings_modal_is_saving: bool,
    pub settings_modal_error: Option<String>,
    pub pending_settings_validation_generation: Option<u64>,
    pub last_error: Option<String>,
    pub last_warning: Option<String>,
    pub deleted_paths: HashSet<PathBuf>,
    pub llm_enabled: bool,
    pub llm_status: LlmStatus,
    pub llm_online: bool,
    pub llm_connection_generation: u64,
    // Scan progress (live during scan)
    pub files_scanned: u64,
    pub bytes_found: u64,
    pub logical_bytes_found: u64,
    pub physical_bytes_found: u64,
    pub current_scan_dir: String,
    pub applied_scan_profile_name: Option<String>,
    // LLM classification tracking
    pub llm_classified_count: u64,
    pub llm_pending_count: u64,
    // Precomputed total sizes: path → (logical_total, physical_total)
    // Populated once after scan completes. O(1) lookup instead of O(subtree).
    pub size_cache: HashMap<PathBuf, (u64, u64)>,
    // Sorted indices cache: path → sorted child indices. Populated before each draw.
    pub sorted_cache: HashMap<PathBuf, Vec<usize>>,
    // Preview analytics cache for the selected directory.
    pub preview_cache: Option<PreviewAnalytics>,
    pub preview_cache_path: Option<PathBuf>,
    // Batch review scroll position
    pub batch_review_selected: usize,
    // Directory picker
    pub dir_picker_options: Vec<PathBuf>,
    pub dir_picker_selected: usize,
    pub dir_picker_custom: String,
    pub dir_picker_typing: bool,
}

impl App {
    pub fn new(scan_path: Option<PathBuf>, llm_enabled: bool, preferences: AppConfig) -> Self {
        let screen = if scan_path.is_some() {
            AppScreen::Main
        } else {
            AppScreen::DirPicker
        };
        let sort_key = preferences.ui.sort_key;
        let dir_picker_options = build_dir_picker_options();
        let root = scan_path
            .clone()
            .or_else(|| preferences.ui.last_scan_path.clone())
            .unwrap_or_else(|| PathBuf::from("/"));

        Self {
            screen,
            entries: Vec::new(),
            columns: ColumnStack::new(root.clone(), sort_key),
            marks: MarkSet::new(),
            preview_mode: PreviewMode::Analytics,
            scan_status: ScanStatus::Idle,
            total_size: 0,
            total_logical_size: 0,
            total_physical_size: 0,
            total_files: 0,
            skipped: 0,
            delete_stats: DeleteOutcome::default(),
            scan_path: root,
            should_quit: false,
            preferences,
            settings_modal_is_saving: false,
            settings_modal_error: None,
            pending_settings_validation_generation: None,
            last_error: None,
            last_warning: None,
            deleted_paths: HashSet::new(),
            llm_enabled,
            llm_status: if llm_enabled {
                LlmStatus::NeedsSetup
            } else {
                LlmStatus::Disabled
            },
            llm_online: false,
            llm_connection_generation: 0,
            files_scanned: 0,
            bytes_found: 0,
            logical_bytes_found: 0,
            physical_bytes_found: 0,
            current_scan_dir: String::new(),
            applied_scan_profile_name: None,
            llm_classified_count: 0,
            llm_pending_count: 0,
            size_cache: HashMap::new(),
            sorted_cache: HashMap::new(),
            preview_cache: None,
            preview_cache_path: None,
            batch_review_selected: 0,
            dir_picker_options,
            dir_picker_selected: 0,
            dir_picker_custom: String::new(),
            dir_picker_typing: false,
        }
    }

    // -- Settings and modal management --

    pub fn open_settings(&mut self) {
        self.settings_modal_is_saving = false;
        self.settings_modal_error = None;
        self.pending_settings_validation_generation = None;
        self.preview_mode = PreviewMode::Settings(self.build_settings_draft());
    }

    pub fn open_onboarding(&mut self) {
        self.settings_modal_is_saving = false;
        self.settings_modal_error = None;
        self.pending_settings_validation_generation = None;
        self.preview_mode = PreviewMode::Onboarding(self.build_settings_draft());
    }

    pub fn close_preview_modal(&mut self) {
        self.preview_mode = PreviewMode::Analytics;
        self.settings_modal_is_saving = false;
        self.settings_modal_error = None;
        self.pending_settings_validation_generation = None;
    }

    pub fn build_settings_draft(&self) -> SettingsDraft {
        let provider = match self.preferences.llm.active_provider {
            ProviderKind::Ollama => ProviderKind::OpenRouter,
            provider => provider,
        };
        let settings = self
            .preferences
            .llm
            .providers
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| default_provider_settings(provider));

        SettingsDraft {
            provider,
            api_key: String::new(),
            api_key_edited: false,
            api_key_editing: false,
            model: settings.model,
            base_url: settings.base_url,
            llm_enabled: self.preferences.llm.enabled,
            size_mode: self.size_mode(),
            selected_scan_profile: self
                .preferences
                .active_scan_profile()
                .map(|profile| profile.name.clone()),
        }
    }

    // -- Scan management --

    pub fn start_scan_with_path(&mut self, path: PathBuf) {
        self.preferences.ui.last_scan_path = Some(path.clone());
        self.scan_path = path.clone();
        self.screen = AppScreen::Main;
        self.scan_status = ScanStatus::Scanning;
        self.entries.clear();
        self.columns = ColumnStack::new(path, self.columns.sort_key);
        self.marks.clear();
        self.preview_mode = PreviewMode::Analytics;
        self.total_size = 0;
        self.total_logical_size = 0;
        self.total_physical_size = 0;
        self.total_files = 0;
        self.skipped = 0;
        self.delete_stats = DeleteOutcome::default();
        self.files_scanned = 0;
        self.bytes_found = 0;
        self.logical_bytes_found = 0;
        self.physical_bytes_found = 0;
        self.current_scan_dir.clear();
        self.applied_scan_profile_name = None;
        self.last_error = None;
        self.last_warning = None;
        self.deleted_paths.clear();
        self.llm_classified_count = 0;
        self.llm_pending_count = 0;
        self.sorted_cache.clear();
        self.preview_cache = None;
        self.preview_cache_path = None;
    }

    // -- Size mode --

    pub fn size_mode(&self) -> SizeMode {
        self.preferences.ui.size_mode
    }

    #[allow(dead_code)] // May be used by status bar or scan display
    pub fn active_scan_profile_name(&self) -> Option<&str> {
        match self.scan_status {
            ScanStatus::Scanning | ScanStatus::Complete => {
                self.applied_scan_profile_name.as_deref()
            }
            ScanStatus::Idle => self
                .preferences
                .active_scan_profile()
                .map(|profile| profile.name.as_str()),
        }
    }

    pub fn sync_display_size_state(&mut self) {
        self.total_size = match self.size_mode() {
            SizeMode::Physical => self.total_physical_size,
            SizeMode::Logical => self.total_logical_size,
        };
        self.bytes_found = match self.size_mode() {
            SizeMode::Physical => self.physical_bytes_found,
            SizeMode::Logical => self.logical_bytes_found,
        };
    }

    // -- Cache invalidation --

    /// Clear all render caches. Call after: sort key change, size mode change,
    /// scan complete, deletion, LLM classification update.
    pub fn invalidate_caches(&mut self) {
        self.sorted_cache.clear();
        self.preview_cache = None;
        self.preview_cache_path = None;
    }

    /// Clear just the preview cache. Call after: j/k movement, h/l navigation.
    pub fn invalidate_preview_cache(&mut self) {
        self.preview_cache = None;
        self.preview_cache_path = None;
    }

    /// Precompute sorted indices and preview analytics for the current frame.
    /// Called once before draw. Render functions do pure O(1) lookups after this.
    pub fn refresh_frame_cache(&mut self) {
        let sort_key = self.columns.sort_key;
        let mode = self.size_mode();

        // Cap sorted_cache size to prevent unbounded growth
        if self.sorted_cache.len() > 16 {
            self.sorted_cache.clear();
        }

        // Parent column
        if let Some(parent) = self.columns.parent() {
            let parent_path = parent.path.clone();
            if !self.sorted_cache.contains_key(&parent_path) {
                if let Some(children) = self.children_at_path(&parent_path) {
                    let cache = &self.size_cache;
                    let sorted = sorted_children_cached(children, sort_key, |e| {
                        cache.get(&e.path).map(|&(l, p)| match mode {
                            SizeMode::Logical => l,
                            SizeMode::Physical => p,
                        }).unwrap_or(0)
                    });
                    self.sorted_cache.insert(parent_path, sorted);
                }
            }
        }

        // Current column
        let current_path = self.columns.current_path().to_path_buf();
        if !self.sorted_cache.contains_key(&current_path) {
            if let Some(children) = self.children_at_path(&current_path) {
                let cache = &self.size_cache;
                let sorted = sorted_children_cached(children, sort_key, |e| {
                    cache.get(&e.path).map(|&(l, p)| match mode {
                        SizeMode::Logical => l,
                        SizeMode::Physical => p,
                    }).unwrap_or(0)
                });
                self.sorted_cache.insert(current_path.clone(), sorted);
            }
        }

        // Preview analytics for selected entry
        if let Some(sorted) = self.sorted_cache.get(&current_path).cloned() {
            let sel_idx = self.columns.current().selected_index;
            if let Some(&idx) = sorted.get(sel_idx) {
                if let Some(children) = self.children_at_path(&current_path) {
                    if let Some(entry) = children.get(idx) {
                        let selected_path = entry.path.clone();
                        let is_dir = entry.is_dir;

                        if self.preview_cache_path.as_ref() != Some(&selected_path) {
                            if is_dir {
                                if let Some(sel_children) = self.children_at_path(&selected_path) {
                                    let by_category = aggregate_by_category(sel_children, &self.size_cache, mode);
                                    let by_age = aggregate_by_age(sel_children);
                                    self.preview_cache = Some(PreviewAnalytics { by_category, by_age });
                                } else {
                                    self.preview_cache = None;
                                }
                            } else {
                                self.preview_cache = None;
                            }
                            self.preview_cache_path = Some(selected_path);
                        }
                    }
                }
            }
        }
    }

    /// O(1) lookup of cached sorted indices for a directory.
    pub fn get_sorted_children(&self, path: &Path) -> Option<&[usize]> {
        self.sorted_cache.get(path).map(|v| v.as_slice())
    }

    // -- Size cache --

    /// Walk the entry tree once and cache total sizes for every node.
    /// Call after scan completes or after deletion changes the tree.
    pub fn rebuild_size_cache(&mut self) {
        self.size_cache.clear();
        for entry in &self.entries {
            cache_entry_sizes(entry, &mut self.size_cache);
        }
    }

    /// O(1) cached total size for a path. Falls back to entry's own size if not cached.
    pub fn cached_size(&self, path: &Path, mode: SizeMode) -> u64 {
        if let Some(&(logical, physical)) = self.size_cache.get(path) {
            match mode {
                SizeMode::Logical => logical,
                SizeMode::Physical => physical,
            }
        } else {
            0
        }
    }

    // -- Column navigation helpers --

    /// Get the children of the directory at `path` in the entry tree.
    pub fn children_at_path(&self, path: &Path) -> Option<&[FileEntry]> {
        // If path is the scan root, the top-level entries are the children
        if path == self.scan_path {
            return Some(&self.entries);
        }
        find_children(&self.entries, path)
    }

    /// Look up a single entry by path in the tree.
    pub fn entry_at_path(&self, path: &Path) -> Option<&FileEntry> {
        find_entry(&self.entries, path)
    }

    /// Get the entry currently highlighted in the current column.
    /// Uses sorted_cache for O(1) lookup when cache is warm (after refresh_frame_cache).
    pub fn selected_entry(&self) -> Option<&FileEntry> {
        let col = self.columns.current();
        let children = self.children_at_path(&col.path)?;

        // Use cached sorted indices if available (fast path after refresh_frame_cache)
        if let Some(sorted) = self.sorted_cache.get(&col.path) {
            let idx = sorted.get(col.selected_index)?;
            return children.get(*idx);
        }

        // Fallback: compute inline (cold cache, e.g. during tests)
        let cache = &self.size_cache;
        let mode = self.size_mode();
        let sorted = sorted_children_cached(children, self.columns.sort_key, |e| {
            cache.get(&e.path).map(|&(l, p)| match mode {
                SizeMode::Logical => l,
                SizeMode::Physical => p,
            }).unwrap_or(0)
        });
        let idx = sorted.get(col.selected_index)?;
        children.get(*idx)
    }

    /// Get the path of the currently selected entry.
    pub fn selected_path(&self) -> Option<PathBuf> {
        self.selected_entry().map(|e| e.path.clone())
    }

    /// How many children the current column has.
    pub fn current_children_count(&self) -> usize {
        self.children_at_path(self.columns.current_path())
            .map_or(0, |c| c.len())
    }

    /// How many children the parent column has.
    #[allow(dead_code)] // Used by status_bar once Task 10 is done
    pub fn parent_children_count(&self) -> usize {
        self.columns
            .parent()
            .and_then(|parent| self.children_at_path(&parent.path))
            .map_or(0, |c| c.len())
    }

    // -- Entry mutation --

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
        self.marks.remove(path);
    }

    /// Update an entry's classification in-place.
    #[allow(dead_code)] // Used by LLM result application
    pub fn update_entry_classification(
        &mut self,
        path: &Path,
        category: Category,
        safety: SafetyLevel,
        reason: String,
    ) -> bool {
        fn update(
            entries: &mut [FileEntry],
            path: &Path,
            category: Category,
            safety: SafetyLevel,
            reason: &str,
        ) -> bool {
            for entry in entries {
                if entry.path == path {
                    entry.category = category;
                    entry.safety = safety;
                    entry.safety_reason = reason.to_string();
                    return true;
                }
                if update(&mut entry.children, path, category, safety, reason) {
                    return true;
                }
            }
            false
        }
        update(&mut self.entries, path, category, safety, &reason)
    }

    /// Ensure scroll_offset keeps the selected index visible.
    #[allow(dead_code)] // Used by column rendering in draw path
    pub fn ensure_visible(&mut self, area_height: u16) {
        let col = self.columns.current_mut();
        let h = area_height as usize;
        if h == 0 {
            return;
        }
        if col.selected_index < col.scroll_offset {
            col.scroll_offset = col.selected_index;
        } else if col.selected_index >= col.scroll_offset + h {
            col.scroll_offset = col.selected_index.saturating_sub(h - 1);
        }
    }
}

/// Recursively cache total sizes for an entry and all descendants. Returns (logical, physical).
fn cache_entry_sizes(entry: &FileEntry, cache: &mut HashMap<PathBuf, (u64, u64)>) -> (u64, u64) {
    if entry.children.is_empty() {
        let logical = entry.sizes.accounted_total_bytes(SizeMode::Logical);
        let physical = entry.sizes.accounted_total_bytes(SizeMode::Physical);
        cache.insert(entry.path.clone(), (logical, physical));
        (logical, physical)
    } else {
        let (mut total_logical, mut total_physical) = (0u64, 0u64);
        for child in &entry.children {
            let (cl, cp) = cache_entry_sizes(child, cache);
            total_logical += cl;
            total_physical += cp;
        }
        cache.insert(entry.path.clone(), (total_logical, total_physical));
        (total_logical, total_physical)
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
    use crate::config::AppConfig;
    use purifier_core::provider::ProviderKind;
    use std::path::PathBuf;

    fn dir(path: &str, children: Vec<FileEntry>) -> FileEntry {
        let mut entry = FileEntry::new(PathBuf::from(path), 0, true, None);
        entry.children = children;
        entry
    }

    fn file(path: &str, size: u64) -> FileEntry {
        FileEntry::new(PathBuf::from(path), size, false, None)
    }

    #[test]
    fn start_scan_with_path_should_record_last_scan_path_for_persistence() {
        let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());

        app.start_scan_with_path(PathBuf::from("/tmp/project"));

        assert_eq!(
            app.preferences.ui.last_scan_path,
            Some(PathBuf::from("/tmp/project"))
        );
    }

    #[test]
    fn onboarding_should_default_to_openrouter_when_no_provider_is_configured() {
        let mut app = App::new(Some(PathBuf::from("/")), true, AppConfig::default());
        app.open_onboarding();

        match &app.preview_mode {
            PreviewMode::Onboarding(draft) => {
                assert_eq!(draft.provider, ProviderKind::OpenRouter)
            }
            other => panic!("expected onboarding preview, got {other:?}"),
        }
    }

    #[test]
    fn children_at_path_returns_top_level_entries_for_scan_root() {
        let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
        app.entries = vec![
            file("/a", 10),
            file("/b", 20),
        ];

        let children = app.children_at_path(Path::new("/")).unwrap();
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn children_at_path_returns_nested_children() {
        let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
        app.entries = vec![dir(
            "/root",
            vec![file("/root/a", 10), file("/root/b", 20)],
        )];

        let children = app.children_at_path(Path::new("/root")).unwrap();
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn selected_entry_returns_correct_entry() {
        let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
        app.entries = vec![file("/a", 10), file("/b", 30), file("/c", 20)];
        app.rebuild_size_cache();
        // Sort by size: b(30), c(20), a(10) — selected_index=0 → b
        let entry = app.selected_entry().unwrap();
        assert_eq!(entry.path, PathBuf::from("/b"));
    }

    #[test]
    fn update_entry_classification_works() {
        let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
        app.entries = vec![file("/test", 10)];

        assert!(app.update_entry_classification(
            Path::new("/test"),
            Category::Cache,
            SafetyLevel::Safe,
            "test cache".to_string(),
        ));

        let entry = app.entry_at_path(Path::new("/test")).unwrap();
        assert_eq!(entry.category, Category::Cache);
        assert_eq!(entry.safety, SafetyLevel::Safe);
    }

    #[test]
    fn remove_entry_by_path_removes_from_tree() {
        let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
        app.entries = vec![file("/a", 10), file("/b", 20)];

        assert!(app.remove_entry_by_path(Path::new("/a")));
        assert_eq!(app.entries.len(), 1);
        assert_eq!(app.entries[0].path, PathBuf::from("/b"));
    }

    #[test]
    fn open_settings_should_ignore_stale_selected_scan_profile() {
        let mut config = AppConfig::default();
        config.ui.last_selected_scan_profile = Some("missing-profile".to_string());
        config.ui.scan_profiles = vec![purifier_core::ScanProfile {
            name: "existing-profile".to_string(),
            exclude: None,
            mask: None,
            display_filter: None,
        }];
        let mut app = App::new(Some(PathBuf::from("/")), false, config);

        app.open_settings();

        match &app.preview_mode {
            PreviewMode::Settings(draft) => {
                assert_eq!(draft.selected_scan_profile, None);
            }
            _ => panic!("expected settings preview"),
        }
    }
}
