use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use crossbeam_channel::{Receiver, Sender};
use jwalk::WalkDir;

use crate::types::ScanEvent;

pub fn scan(root: &Path) -> Receiver<ScanEvent> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let root = root.to_path_buf();

    std::thread::spawn(move || {
        run_scan(&root, &tx);
    });

    rx
}

fn run_scan(root: &Path, tx: &Sender<ScanEvent>) {
    let total_size = Arc::new(AtomicU64::new(0));
    let total_files = Arc::new(AtomicU64::new(0));
    let skipped = Arc::new(AtomicU64::new(0));

    for entry in WalkDir::new(root).skip_hidden(false).sort(false) {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => {
                        skipped.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                };

                let size = metadata.len();
                let is_dir = metadata.is_dir();
                let modified = metadata.modified().ok();

                total_files.fetch_add(1, Ordering::Relaxed);
                if !is_dir {
                    total_size.fetch_add(size, Ordering::Relaxed);
                }

                let event = ScanEvent::Entry {
                    path,
                    size,
                    is_dir,
                    modified,
                };

                if tx.send(event).is_err() {
                    return; // receiver dropped, stop scanning
                }
            }
            Err(_) => {
                skipped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    let _ = tx.send(ScanEvent::ScanComplete {
        total_size: total_size.load(Ordering::Relaxed),
        total_files: total_files.load(Ordering::Relaxed),
        skipped: skipped.load(Ordering::Relaxed),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Create test structure
        fs::write(root.join("file1.txt"), "hello").unwrap();
        fs::write(root.join("file2.txt"), "world!").unwrap();
        fs::create_dir(root.join("subdir")).unwrap();
        fs::write(root.join("subdir").join("nested.txt"), "nested content").unwrap();

        let rx = scan(root);

        let mut entries = Vec::new();
        let mut complete = None;

        for event in rx {
            match event {
                ScanEvent::Entry { path, size, is_dir, .. } => {
                    entries.push((path, size, is_dir));
                }
                ScanEvent::ScanComplete { total_size, total_files, skipped } => {
                    complete = Some((total_size, total_files, skipped));
                }
            }
        }

        // Should have: root dir, file1.txt, file2.txt, subdir, subdir/nested.txt
        assert!(entries.len() >= 4, "Expected at least 4 entries, got {}", entries.len());

        let (total_size, _total_files, skipped) = complete.expect("Should receive ScanComplete");
        // file1.txt=5 + file2.txt=6 + nested.txt=14 = 25
        assert_eq!(total_size, 25);
        assert_eq!(skipped, 0);
    }
}
