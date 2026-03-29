use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
    pub size: u64,
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
            size,
            is_dir,
            modified,
            category: Category::Unknown,
            safety: SafetyLevel::Unknown,
            safety_reason: String::new(),
            children: Vec::new(),
            expanded: false,
        }
    }

    pub fn total_size(&self) -> u64 {
        if self.children.is_empty() {
            self.size
        } else {
            self.children.iter().map(|c| c.total_size()).sum()
        }
    }
}

#[derive(Debug, Clone)]
pub enum ScanEvent {
    Entry {
        path: PathBuf,
        size: u64,
        is_dir: bool,
        modified: Option<SystemTime>,
    },
    Progress {
        files_scanned: u64,
        bytes_found: u64,
        current_dir: String,
    },
    ScanComplete {
        total_size: u64,
        total_files: u64,
        skipped: u64,
    },
}
