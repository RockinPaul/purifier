use std::collections::HashSet;
use std::path::{Path, PathBuf};

use purifier_core::size::SizeMode;
use purifier_core::types::FileEntry;

/// Tracks paths marked for batch deletion.
#[derive(Debug, Clone, Default)]
pub struct MarkSet {
    marked: HashSet<PathBuf>,
}

impl MarkSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle(&mut self, path: &Path) {
        let path = path.to_path_buf();
        if !self.marked.remove(&path) {
            self.marked.insert(path);
        }
    }

    pub fn is_marked(&self, path: &Path) -> bool {
        self.marked.contains(path)
    }

    pub fn clear(&mut self) {
        self.marked.clear();
    }

    pub fn count(&self) -> usize {
        self.marked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marked.is_empty()
    }

    pub fn remove(&mut self, path: &Path) {
        self.marked.remove(path);
    }

    /// Sum sizes of all marked entries by walking the entry tree.
    #[allow(dead_code)] // Used in tests; UI uses cached_size instead
    pub fn total_size(&self, entries: &[FileEntry], mode: SizeMode) -> u64 {
        self.marked
            .iter()
            .filter_map(|path| find_entry_size(entries, path, mode))
            .sum()
    }

    #[allow(dead_code)] // Available for batch review iteration
    pub fn iter(&self) -> impl Iterator<Item = &PathBuf> {
        self.marked.iter()
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<_> = self.marked.iter().cloned().collect();
        paths.sort();
        paths
    }
}

#[allow(dead_code)]
fn find_entry_size(entries: &[FileEntry], path: &Path, mode: SizeMode) -> Option<u64> {
    for entry in entries {
        if entry.path == path {
            return Some(entry.total_size(mode));
        }
        if let Some(size) = find_entry_size(&entry.children, path, mode) {
            return Some(size);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_file(path: &str, size: u64) -> FileEntry {
        FileEntry::new(PathBuf::from(path), size, false, None)
    }

    fn make_dir(path: &str, children: Vec<FileEntry>) -> FileEntry {
        let mut entry = FileEntry::new(PathBuf::from(path), 0, true, None);
        entry.children = children;
        entry
    }

    #[test]
    fn toggle_adds_then_removes() {
        let mut marks = MarkSet::new();
        let path = Path::new("/tmp/file");

        marks.toggle(path);
        assert!(marks.is_marked(path));
        assert_eq!(marks.count(), 1);

        marks.toggle(path);
        assert!(!marks.is_marked(path));
        assert_eq!(marks.count(), 0);
    }

    #[test]
    fn clear_empties_the_set() {
        let mut marks = MarkSet::new();
        marks.toggle(Path::new("/a"));
        marks.toggle(Path::new("/b"));
        assert_eq!(marks.count(), 2);

        marks.clear();
        assert_eq!(marks.count(), 0);
        assert!(marks.is_empty());
    }

    #[test]
    fn total_size_sums_across_tree() {
        let entries = vec![make_dir(
            "/root",
            vec![make_file("/root/a", 100), make_file("/root/b", 200)],
        )];

        let mut marks = MarkSet::new();
        marks.toggle(Path::new("/root/a"));
        marks.toggle(Path::new("/root/b"));

        let total = marks.total_size(&entries, SizeMode::Logical);
        assert_eq!(total, 300);
    }

    #[test]
    fn total_size_ignores_unmarked() {
        let entries = vec![make_file("/a", 100), make_file("/b", 200)];

        let mut marks = MarkSet::new();
        marks.toggle(Path::new("/a"));

        let total = marks.total_size(&entries, SizeMode::Logical);
        assert_eq!(total, 100);
    }

    #[test]
    fn remove_unmarks_individual() {
        let mut marks = MarkSet::new();
        marks.toggle(Path::new("/a"));
        marks.toggle(Path::new("/b"));

        marks.remove(Path::new("/a"));
        assert!(!marks.is_marked(Path::new("/a")));
        assert!(marks.is_marked(Path::new("/b")));
    }

    #[test]
    fn paths_returns_sorted() {
        let mut marks = MarkSet::new();
        marks.toggle(Path::new("/c"));
        marks.toggle(Path::new("/a"));
        marks.toggle(Path::new("/b"));

        assert_eq!(
            marks.paths(),
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c"),
            ]
        );
    }
}
