use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::size::{EntrySizes, FileIdentity, SizeMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Category {
    BuildArtifact,
    Cache,
    Download,
    AppData,
    Media,
    System,
    Unknown,
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Category::BuildArtifact => write!(f, "Build Artifact"),
            Category::Cache => write!(f, "Cache"),
            Category::Download => write!(f, "Download"),
            Category::AppData => write!(f, "App Data"),
            Category::Media => write!(f, "Media"),
            Category::System => write!(f, "System"),
            Category::Unknown => write!(f, "Unknown"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SafetyLevel {
    Safe,
    Caution,
    Unsafe,
    Unknown,
}

impl std::fmt::Display for SafetyLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SafetyLevel::Safe => write!(f, "Safe"),
            SafetyLevel::Caution => write!(f, "Caution"),
            SafetyLevel::Unsafe => write!(f, "Unsafe"),
            SafetyLevel::Unknown => write!(f, "Unknown"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub sizes: EntrySizes,
    pub file_identity: Option<FileIdentity>,
    pub is_dir: bool,
    pub modified: Option<SystemTime>,
    pub category: Category,
    pub safety: SafetyLevel,
    pub safety_reason: String,
    pub children: Vec<FileEntry>,
    pub expanded: bool,
}

impl FileEntry {
    pub fn new(path: PathBuf, size: u64, is_dir: bool, modified: Option<SystemTime>) -> Self {
        Self {
            path,
            sizes: EntrySizes {
                logical_bytes: size,
                physical_bytes: size,
                accounted_physical_bytes: size,
            },
            file_identity: None,
            is_dir,
            modified,
            category: Category::Unknown,
            safety: SafetyLevel::Unknown,
            safety_reason: String::new(),
            children: Vec::new(),
            expanded: false,
        }
    }

    pub fn new_with_sizes(
        path: PathBuf,
        sizes: EntrySizes,
        file_identity: Option<FileIdentity>,
        is_dir: bool,
        modified: Option<SystemTime>,
    ) -> Self {
        Self {
            path,
            sizes,
            file_identity,
            is_dir,
            modified,
            category: Category::Unknown,
            safety: SafetyLevel::Unknown,
            safety_reason: String::new(),
            children: Vec::new(),
            expanded: false,
        }
    }

    pub fn total_size(&self, mode: SizeMode) -> u64 {
        if self.children.is_empty() {
            self.sizes.accounted_total_bytes(mode)
        } else {
            self.children.iter().map(|c| c.total_size(mode)).sum()
        }
    }
}

#[derive(Debug, Clone)]
pub enum ScanEvent {
    Entry {
        path: PathBuf,
        sizes: EntrySizes,
        file_identity: Option<FileIdentity>,
        is_dir: bool,
        modified: Option<SystemTime>,
    },
    Progress {
        entries_scanned: u64,
        logical_bytes_found: u64,
        physical_bytes_found: u64,
        current_path: String,
    },
    ScanComplete {
        total_entries: u64,
        total_logical_bytes: u64,
        total_physical_bytes: u64,
        skipped: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_total_size_should_use_accounted_physical_bytes_for_leaf_entries() {
        let entry = FileEntry::new_with_sizes(
            PathBuf::from("/tmp/file"),
            EntrySizes {
                logical_bytes: 100,
                physical_bytes: 4096,
                accounted_physical_bytes: 0,
            },
            None,
            false,
            None,
        );

        assert_eq!(entry.total_size(SizeMode::Physical), 0);
    }

    #[test]
    fn new_with_sizes_should_preserve_identity_and_sizes() {
        let sizes = EntrySizes {
            logical_bytes: 10,
            physical_bytes: 4096,
            accounted_physical_bytes: 4096,
        };
        let identity = FileIdentity {
            dev: 7,
            ino: 42,
            nlink: 3,
        };

        let entry = FileEntry::new_with_sizes(
            PathBuf::from("/tmp/file"),
            sizes,
            Some(identity),
            false,
            None,
        );

        assert_eq!(entry.sizes, sizes);
        assert_eq!(entry.file_identity, Some(identity));
    }
}
