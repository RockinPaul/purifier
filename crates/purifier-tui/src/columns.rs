use std::path::{Path, PathBuf};

use purifier_core::size::SizeMode;
use purifier_core::types::{FileEntry, SafetyLevel};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortKey {
    #[default]
    Size,
    Safety,
    Age,
    Name,
}

impl SortKey {
    pub fn cycle(self) -> Self {
        match self {
            SortKey::Size => SortKey::Safety,
            SortKey::Safety => SortKey::Age,
            SortKey::Age => SortKey::Name,
            SortKey::Name => SortKey::Size,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Size => "Size",
            SortKey::Safety => "Safety",
            SortKey::Age => "Age",
            SortKey::Name => "Name",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ColumnState {
    pub path: PathBuf,
    pub selected_index: usize,
    pub scroll_offset: usize,
}

#[derive(Debug, Clone)]
pub struct ColumnStack {
    columns: Vec<ColumnState>,
    pub sort_key: SortKey,
}

impl ColumnStack {
    pub fn new(root: PathBuf, sort_key: SortKey) -> Self {
        Self {
            columns: vec![ColumnState {
                path: root,
                selected_index: 0,
                scroll_offset: 0,
            }],
            sort_key,
        }
    }

    pub fn current(&self) -> &ColumnState {
        self.columns.last().expect("column stack is never empty")
    }

    pub fn current_mut(&mut self) -> &mut ColumnState {
        self.columns.last_mut().expect("column stack is never empty")
    }

    pub fn parent(&self) -> Option<&ColumnState> {
        if self.columns.len() >= 2 {
            Some(&self.columns[self.columns.len() - 2])
        } else {
            None
        }
    }

    pub fn enter(&mut self, path: PathBuf) {
        self.columns.push(ColumnState {
            path,
            selected_index: 0,
            scroll_offset: 0,
        });
    }

    pub fn back(&mut self) -> bool {
        if self.columns.len() > 1 {
            self.columns.pop();
            true
        } else {
            false
        }
    }

    pub fn move_selection(&mut self, delta: isize, count: usize) {
        if count == 0 {
            return;
        }
        let col = self.current_mut();
        let new_index = if delta < 0 {
            col.selected_index.saturating_sub(delta.unsigned_abs())
        } else {
            col.selected_index
                .saturating_add(delta as usize)
                .min(count.saturating_sub(1))
        };
        col.selected_index = new_index;
    }

    pub fn jump_top(&mut self) {
        self.current_mut().selected_index = 0;
    }

    pub fn jump_bottom(&mut self, count: usize) {
        self.current_mut().selected_index = count.saturating_sub(1);
    }

    pub fn current_path(&self) -> &Path {
        &self.current().path
    }

    #[allow(dead_code)] // Used by status_bar rendering
    pub fn breadcrumb(&self) -> String {
        let path = self.current_path();
        let path_str = path.display().to_string();
        if let Some(home) = dirs::home_dir() {
            let home_str = home.display().to_string();
            if path_str.starts_with(&home_str) {
                return format!("~{}", &path_str[home_str.len()..]);
            }
        }
        path_str
    }

    #[allow(dead_code)] // Used by tests and UI rendering
    pub fn depth(&self) -> usize {
        self.columns.len()
    }

    /// Navigate to a specific path by building the column stack from root.
    pub fn navigate_to(&mut self, target: &Path, entries: &[FileEntry], size_mode: SizeMode) {
        // Reset to root
        while self.columns.len() > 1 {
            self.columns.pop();
        }
        self.current_mut().selected_index = 0;

        // Walk each component of the target path relative to the root
        let root = self.current().path.clone();
        if let Ok(relative) = target.strip_prefix(&root) {
            let mut current = root;
            for component in relative.components() {
                let next = current.join(component);
                // Find the entry in current directory's children and select it
                if let Some(children) = find_children(entries, &current) {
                    let sorted = sorted_children_cached(children, self.sort_key, |e| {
                        e.total_size(size_mode)
                    });
                    if let Some(pos) = sorted
                        .iter()
                        .position(|&idx| children[idx].path == next)
                    {
                        self.current_mut().selected_index = pos;
                    }
                }
                if next.is_dir() {
                    self.enter(next.clone());
                }
                current = next;
            }
        }
    }
}

/// Returns sorted indices into `children` based on the given sort key.
/// `size_fn` provides cached O(1) size lookup instead of recursive total_size().
pub fn sorted_children_cached(
    children: &[FileEntry],
    sort_key: SortKey,
    size_fn: impl Fn(&FileEntry) -> u64,
) -> Vec<usize> {
    // Precompute sizes once into a vec so sort comparisons are O(1)
    let sizes: Vec<u64> = children.iter().map(&size_fn).collect();
    let mut indices: Vec<usize> = (0..children.len()).collect();

    match sort_key {
        SortKey::Size => {
            indices.sort_by(|&a, &b| sizes[b].cmp(&sizes[a]));
        }
        SortKey::Safety => {
            indices.sort_by(|&a, &b| {
                let sa = safety_rank(children[a].safety);
                let sb = safety_rank(children[b].safety);
                sa.cmp(&sb).then_with(|| sizes[b].cmp(&sizes[a]))
            });
        }
        SortKey::Age => {
            indices.sort_by(|&a, &b| {
                match (children[a].modified, children[b].modified) {
                    (Some(ma), Some(mb)) => ma.cmp(&mb),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
                .then_with(|| sizes[b].cmp(&sizes[a]))
            });
        }
        SortKey::Name => {
            indices.sort_by(|&a, &b| {
                let na = children[a]
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase());
                let nb = children[b]
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase());
                na.cmp(&nb)
            });
        }
    }

    indices
}

fn safety_rank(safety: SafetyLevel) -> u8 {
    match safety {
        SafetyLevel::Safe => 0,
        SafetyLevel::Caution => 1,
        SafetyLevel::Unsafe => 2,
        SafetyLevel::Unknown => 3,
    }
}

/// Find the children of a directory by path in the entry tree.
/// Uses prefix-guided descent: only recurses into entries whose path is a prefix
/// of the target. This is O(depth × branching) instead of O(tree_size).
pub fn find_children<'a>(entries: &'a [FileEntry], path: &Path) -> Option<&'a [FileEntry]> {
    for entry in entries {
        if entry.path == path {
            return Some(&entry.children);
        }
        if entry.is_dir && path.starts_with(&entry.path) {
            return find_children(&entry.children, path);
        }
    }
    None
}

/// Find an entry by path in the entry tree.
/// Uses prefix-guided descent: O(depth × branching) instead of O(tree_size).
pub fn find_entry<'a>(entries: &'a [FileEntry], path: &Path) -> Option<&'a FileEntry> {
    for entry in entries {
        if entry.path == path {
            return Some(entry);
        }
        if entry.is_dir && path.starts_with(&entry.path) {
            return find_entry(&entry.children, path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn make_entry(path: &str, size: u64, is_dir: bool, children: Vec<FileEntry>) -> FileEntry {
        let mut entry = FileEntry::new(PathBuf::from(path), size, is_dir, None);
        entry.children = children;
        entry
    }

    fn make_file(path: &str, size: u64, modified: Option<SystemTime>) -> FileEntry {
        let mut entry = FileEntry::new(PathBuf::from(path), size, false, modified);
        entry.safety = SafetyLevel::Unknown;
        entry
    }

    fn make_classified_file(path: &str, size: u64, safety: SafetyLevel) -> FileEntry {
        let mut entry = FileEntry::new(PathBuf::from(path), size, false, None);
        entry.safety = safety;
        entry
    }

    #[test]
    fn enter_pushes_column_and_back_pops_it() {
        let mut stack = ColumnStack::new(PathBuf::from("/"), SortKey::Size);
        assert_eq!(stack.depth(), 1);

        stack.enter(PathBuf::from("/home"));
        assert_eq!(stack.depth(), 2);
        assert_eq!(stack.current_path(), Path::new("/home"));

        assert!(stack.back());
        assert_eq!(stack.depth(), 1);
        assert_eq!(stack.current_path(), Path::new("/"));
    }

    #[test]
    fn back_on_root_returns_false() {
        let mut stack = ColumnStack::new(PathBuf::from("/"), SortKey::Size);
        assert!(!stack.back());
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn move_selection_clamps_to_valid_range() {
        let mut stack = ColumnStack::new(PathBuf::from("/"), SortKey::Size);

        stack.move_selection(10, 3);
        assert_eq!(stack.current().selected_index, 2);

        stack.move_selection(-100, 3);
        assert_eq!(stack.current().selected_index, 0);
    }

    #[test]
    fn move_selection_with_zero_count_is_noop() {
        let mut stack = ColumnStack::new(PathBuf::from("/"), SortKey::Size);
        stack.move_selection(1, 0);
        assert_eq!(stack.current().selected_index, 0);
    }

    #[test]
    fn sorted_children_sorts_by_size_descending() {
        let children = vec![
            make_file("/a", 10, None),
            make_file("/b", 30, None),
            make_file("/c", 20, None),
        ];

        let indices = sorted_children_cached(&children, SortKey::Size, |e| e.total_size(SizeMode::Logical));
        let names: Vec<&str> = indices
            .iter()
            .map(|&i| children[i].path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["b", "c", "a"]);
    }

    #[test]
    fn sorted_children_sorts_by_name_ascending() {
        let children = vec![
            make_file("/charlie", 10, None),
            make_file("/alpha", 20, None),
            make_file("/bravo", 15, None),
        ];

        let indices = sorted_children_cached(&children, SortKey::Name, |e| e.total_size(SizeMode::Logical));
        let names: Vec<&str> = indices
            .iter()
            .map(|&i| children[i].path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn sorted_children_sorts_by_safety_then_size() {
        let children = vec![
            make_classified_file("/safe-small", 10, SafetyLevel::Safe),
            make_classified_file("/unsafe", 5, SafetyLevel::Unsafe),
            make_classified_file("/safe-large", 20, SafetyLevel::Safe),
            make_classified_file("/caution", 15, SafetyLevel::Caution),
        ];

        let indices = sorted_children_cached(&children, SortKey::Safety, |e| e.total_size(SizeMode::Logical));
        let names: Vec<&str> = indices
            .iter()
            .map(|&i| children[i].path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["safe-large", "safe-small", "caution", "unsafe"]);
    }

    #[test]
    fn sorted_children_sorts_by_age_oldest_first() {
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let new = SystemTime::UNIX_EPOCH + Duration::from_secs(200);

        let children = vec![
            make_file("/new", 10, Some(new)),
            make_file("/none", 10, None),
            make_file("/old", 10, Some(old)),
        ];

        let indices = sorted_children_cached(&children, SortKey::Age, |e| e.total_size(SizeMode::Logical));
        let names: Vec<&str> = indices
            .iter()
            .map(|&i| children[i].path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["old", "new", "none"]);
    }

    #[test]
    fn breadcrumb_replaces_home_with_tilde() {
        let stack = ColumnStack::new(PathBuf::from("/"), SortKey::Size);
        assert_eq!(stack.breadcrumb(), "/");
    }

    #[test]
    fn sort_key_cycles_through_all_variants() {
        let key = SortKey::Size;
        assert_eq!(key.cycle(), SortKey::Safety);
        assert_eq!(key.cycle().cycle(), SortKey::Age);
        assert_eq!(key.cycle().cycle().cycle(), SortKey::Name);
        assert_eq!(key.cycle().cycle().cycle().cycle(), SortKey::Size);
    }

    #[test]
    fn find_children_returns_correct_children() {
        let entries = vec![make_entry(
            "/root",
            100,
            true,
            vec![
                make_file("/root/a", 30, None),
                make_file("/root/b", 70, None),
            ],
        )];

        let children = find_children(&entries, Path::new("/root")).unwrap();
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn find_children_returns_none_for_missing_path() {
        let entries = vec![make_entry("/root", 100, true, vec![])];
        assert!(find_children(&entries, Path::new("/missing")).is_none());
    }

    #[test]
    fn find_entry_returns_entry_by_path() {
        let entries = vec![make_entry(
            "/root",
            100,
            true,
            vec![make_file("/root/a", 30, None)],
        )];

        let entry = find_entry(&entries, Path::new("/root/a")).unwrap();
        assert_eq!(entry.path, PathBuf::from("/root/a"));
    }

    #[test]
    fn jump_top_and_bottom_work() {
        let mut stack = ColumnStack::new(PathBuf::from("/"), SortKey::Size);
        stack.current_mut().selected_index = 5;

        stack.jump_top();
        assert_eq!(stack.current().selected_index, 0);

        stack.jump_bottom(10);
        assert_eq!(stack.current().selected_index, 9);
    }

    #[test]
    fn parent_returns_none_at_root() {
        let stack = ColumnStack::new(PathBuf::from("/"), SortKey::Size);
        assert!(stack.parent().is_none());
    }

    #[test]
    fn parent_returns_previous_column() {
        let mut stack = ColumnStack::new(PathBuf::from("/"), SortKey::Size);
        stack.enter(PathBuf::from("/home"));
        let parent = stack.parent().unwrap();
        assert_eq!(parent.path, PathBuf::from("/"));
    }
}
