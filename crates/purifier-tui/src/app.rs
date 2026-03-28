use std::path::PathBuf;

use purifier_core::types::{Category, FileEntry, SafetyLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    pub fn index(&self) -> usize {
        match self {
            View::BySize => 0,
            View::ByType => 1,
            View::BySafety => 2,
            View::ByAge => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    Idle,
    Scanning,
    Complete,
}

pub struct App {
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
    pub llm_enabled: bool,
    pub llm_online: bool,
    pub flat_entries: Vec<FlatEntry>,
}

#[derive(Debug, Clone)]
pub struct FlatEntry {
    pub depth: usize,
    pub entry_index: Vec<usize>, // path through the tree
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
    pub expanded: bool,
    pub category: Category,
    pub safety: SafetyLevel,
    pub safety_reason: String,
}

impl App {
    pub fn new(scan_path: PathBuf, llm_enabled: bool) -> Self {
        Self {
            entries: Vec::new(),
            current_view: View::BySize,
            selected_index: 0,
            scan_status: ScanStatus::Idle,
            total_size: 0,
            total_files: 0,
            skipped: 0,
            freed_space: 0,
            scan_path,
            should_quit: false,
            show_delete_confirm: false,
            llm_enabled,
            llm_online: false,
            flat_entries: Vec::new(),
        }
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
                let index_path = flat.entry_index.clone();
                if let Some(entry) = self.get_entry_mut(&index_path) {
                    entry.expanded = !entry.expanded;
                }
                self.rebuild_flat_entries();
            }
        }
    }

    pub fn selected_entry(&self) -> Option<&FlatEntry> {
        self.flat_entries.get(self.selected_index)
    }

    fn get_entry_mut(&mut self, index_path: &[usize]) -> Option<&mut FileEntry> {
        if index_path.is_empty() {
            return None;
        }
        let mut current = self.entries.get_mut(index_path[0])?;
        for &idx in &index_path[1..] {
            current = current.children.get_mut(idx)?;
        }
        Some(current)
    }

    pub fn rebuild_flat_entries(&mut self) {
        self.flat_entries.clear();
        match self.current_view {
            View::BySize => self.flatten_by_size(),
            View::ByType => self.flatten_by_group(|e| e.category),
            View::BySafety => self.flatten_by_group(|e| e.safety),
            View::ByAge => self.flatten_by_size(), // same tree, will be sorted by age in UI
        }
    }

    fn flatten_by_size(&mut self) {
        let mut sorted = self.entries.clone();
        sorted.sort_by(|a, b| b.total_size().cmp(&a.total_size()));

        for (i, entry) in sorted.iter().enumerate() {
            self.flatten_entry(entry, 0, vec![i]);
        }
    }

    fn flatten_entry(&mut self, entry: &FileEntry, depth: usize, index_path: Vec<usize>) {
        self.flat_entries.push(FlatEntry {
            depth,
            entry_index: index_path.clone(),
            path: entry.path.clone(),
            size: entry.total_size(),
            is_dir: entry.is_dir,
            expanded: entry.expanded,
            category: entry.category,
            safety: entry.safety,
            safety_reason: entry.safety_reason.clone(),
        });

        if entry.expanded {
            let mut children: Vec<(usize, &FileEntry)> =
                entry.children.iter().enumerate().collect();
            children.sort_by(|a, b| b.1.total_size().cmp(&a.1.total_size()));

            for (child_idx, child) in children {
                let mut child_path = index_path.clone();
                child_path.push(child_idx);
                self.flatten_entry(child, depth + 1, child_path);
            }
        }
    }

    fn flatten_by_group<K: Ord + std::fmt::Display, F: Fn(&FlatEntry) -> K>(&mut self, key_fn: F) {
        // First build flat list from size view
        let mut all_flat = Vec::new();
        let sorted = {
            let mut s = self.entries.clone();
            s.sort_by(|a, b| b.total_size().cmp(&a.total_size()));
            s
        };
        for (i, entry) in sorted.iter().enumerate() {
            Self::collect_flat(entry, 0, vec![i], &mut all_flat);
        }

        // Group by key
        all_flat.sort_by(|a, b| {
            let ka = key_fn(a);
            let kb = key_fn(b);
            ka.cmp(&kb).then(b.size.cmp(&a.size))
        });

        self.flat_entries = all_flat;
    }

    fn collect_flat(
        entry: &FileEntry,
        depth: usize,
        index_path: Vec<usize>,
        out: &mut Vec<FlatEntry>,
    ) {
        out.push(FlatEntry {
            depth,
            entry_index: index_path.clone(),
            path: entry.path.clone(),
            size: entry.total_size(),
            is_dir: entry.is_dir,
            expanded: entry.expanded,
            category: entry.category,
            safety: entry.safety,
            safety_reason: entry.safety_reason.clone(),
        });

        for (child_idx, child) in entry.children.iter().enumerate() {
            let mut child_path = index_path.clone();
            child_path.push(child_idx);
            Self::collect_flat(child, depth + 1, child_path, out);
        }
    }
}
