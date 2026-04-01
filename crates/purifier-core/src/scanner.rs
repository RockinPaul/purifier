use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[cfg(unix)]
use std::collections::HashSet;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use crossbeam_channel::{Receiver, Sender};
use jwalk::WalkDir;

use crate::filters::{is_package_path, FilterEntryMeta, HardLinkStatus, ScanProfile};
use crate::size::{EntrySizes, FileIdentity};
use crate::types::ScanEvent;

const PROGRESS_INTERVAL: u64 = 500;
#[cfg(unix)]
const STAT_BLOCK_SIZE_BYTES: u64 = 512;

pub fn scan(root: &Path) -> Receiver<ScanEvent> {
    scan_with_profile(root, None)
}

pub fn scan_with_profile(root: &Path, profile: Option<ScanProfile>) -> Receiver<ScanEvent> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let root = root.to_path_buf();

    std::thread::spawn(move || {
        run_scan(&root, &tx, profile.as_ref());
    });

    rx
}

fn run_scan(root: &Path, tx: &Sender<ScanEvent>, profile: Option<&ScanProfile>) {
    let total_logical_bytes = Arc::new(AtomicU64::new(0));
    let total_physical_bytes = Arc::new(AtomicU64::new(0));
    let total_entries = Arc::new(AtomicU64::new(0));
    let skipped = Arc::new(AtomicU64::new(0));
    let mut counter: u64 = 0;
    let mut last_dir = String::new();
    let mut excluded_roots = Vec::new();
    #[cfg(unix)]
    let mut seen_file_ids: HashSet<(u64, u64)> = HashSet::new();
    #[cfg(not(unix))]
    let mut seen_file_ids = ();

    for entry in WalkDir::new(root).skip_hidden(false).sort(false) {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if excluded_roots
                    .iter()
                    .any(|excluded_root: &std::path::PathBuf| path.starts_with(excluded_root))
                {
                    continue;
                }

                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => {
                        skipped.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                };

                let is_dir = metadata.is_dir();
                if profile.is_some_and(|profile| {
                    profile.exclude.as_ref().is_some_and(|filter| {
                        filter.matches(&filter_meta(&path, &metadata, is_dir))
                    })
                }) {
                    if is_dir {
                        excluded_roots.push(path.to_path_buf());
                    }
                    continue;
                }

                let modified = metadata.modified().ok();
                let sizes = file_sizes(&metadata, &mut seen_file_ids, is_dir);
                let file_identity = file_identity(&metadata, is_dir);

                total_entries.fetch_add(1, Ordering::Relaxed);
                total_logical_bytes.fetch_add(sizes.logical_bytes, Ordering::Relaxed);
                total_physical_bytes.fetch_add(sizes.accounted_physical_bytes, Ordering::Relaxed);

                if is_dir {
                    last_dir = path.display().to_string();
                }

                let event = ScanEvent::Entry {
                    path,
                    sizes,
                    file_identity,
                    is_dir,
                    modified,
                };

                if tx.send(event).is_err() {
                    return;
                }

                counter += 1;
                if counter.is_multiple_of(PROGRESS_INTERVAL) {
                    let progress = ScanEvent::Progress {
                        entries_scanned: total_entries.load(Ordering::Relaxed),
                        logical_bytes_found: total_logical_bytes.load(Ordering::Relaxed),
                        physical_bytes_found: total_physical_bytes.load(Ordering::Relaxed),
                        current_path: last_dir.clone(),
                    };
                    if tx.send(progress).is_err() {
                        return;
                    }
                }
            }
            Err(_) => {
                skipped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    let _ = tx.send(ScanEvent::ScanComplete {
        total_entries: total_entries.load(Ordering::Relaxed),
        total_logical_bytes: total_logical_bytes.load(Ordering::Relaxed),
        total_physical_bytes: total_physical_bytes.load(Ordering::Relaxed),
        skipped: skipped.load(Ordering::Relaxed),
    });
}

#[cfg(unix)]
fn file_sizes(
    metadata: &std::fs::Metadata,
    seen_file_ids: &mut HashSet<(u64, u64)>,
    is_dir: bool,
) -> EntrySizes {
    if is_dir {
        return EntrySizes::default();
    }

    let logical_bytes = metadata.len();
    let physical_bytes = metadata.blocks() * STAT_BLOCK_SIZE_BYTES;
    let accounted_physical_bytes = if metadata.nlink() > 1 {
        let file_id = (metadata.dev(), metadata.ino());
        if !seen_file_ids.insert(file_id) {
            0
        } else {
            physical_bytes
        }
    } else {
        physical_bytes
    };

    EntrySizes {
        logical_bytes,
        physical_bytes,
        accounted_physical_bytes,
    }
}

#[cfg(not(unix))]
fn file_sizes(metadata: &std::fs::Metadata, _seen_file_ids: &mut (), is_dir: bool) -> EntrySizes {
    if is_dir {
        return EntrySizes::default();
    }

    let logical_bytes = metadata.len();

    EntrySizes {
        logical_bytes,
        physical_bytes: logical_bytes,
        accounted_physical_bytes: logical_bytes,
    }
}

#[cfg(unix)]
fn file_identity(metadata: &std::fs::Metadata, is_dir: bool) -> Option<FileIdentity> {
    (!is_dir && metadata.nlink() > 1).then(|| FileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        nlink: metadata.nlink(),
    })
}

#[cfg(not(unix))]
fn file_identity(_metadata: &std::fs::Metadata, _is_dir: bool) -> Option<FileIdentity> {
    None
}

#[cfg(unix)]
fn filter_meta(path: &Path, metadata: &std::fs::Metadata, is_dir: bool) -> FilterEntryMeta {
    FilterEntryMeta {
        path: path.to_path_buf(),
        logical_bytes: metadata.len(),
        physical_bytes: if is_dir {
            0
        } else {
            metadata.blocks() * STAT_BLOCK_SIZE_BYTES
        },
        is_dir,
        is_package: is_package_path(path),
        hard_link_status: if !is_dir && metadata.nlink() > 1 {
            HardLinkStatus::IsHardLinked
        } else {
            HardLinkStatus::IsNotHardLinked
        },
    }
}

#[cfg(not(unix))]
fn filter_meta(path: &Path, metadata: &std::fs::Metadata, is_dir: bool) -> FilterEntryMeta {
    FilterEntryMeta {
        path: path.to_path_buf(),
        logical_bytes: metadata.len(),
        physical_bytes: if is_dir { 0 } else { metadata.len() },
        is_dir,
        is_package: is_package_path(path),
        hard_link_status: HardLinkStatus::IsNotHardLinked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use crate::filters::{Filter, FilterTest, ScanProfile};

    #[cfg(unix)]
    use std::fs::File;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn test_scan_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Create test structure
        fs::write(root.join("file1.txt"), "hello").unwrap();
        fs::write(root.join("file2.txt"), "world!").unwrap();
        fs::create_dir(root.join("subdir")).unwrap();
        fs::write(root.join("subdir").join("nested.txt"), "nested content").unwrap();

        #[cfg(unix)]
        let expected_physical_bytes = [
            root.join("file1.txt"),
            root.join("file2.txt"),
            root.join("subdir").join("nested.txt"),
        ]
        .into_iter()
        .map(|path| fs::metadata(path).unwrap().blocks() * STAT_BLOCK_SIZE_BYTES)
        .sum::<u64>();

        let rx = scan(root);

        let mut entries = Vec::new();
        let mut complete = None;

        for event in rx {
            match event {
                ScanEvent::Entry {
                    path,
                    sizes,
                    is_dir,
                    ..
                } => {
                    entries.push((path, sizes.logical_bytes, is_dir));
                }
                ScanEvent::Progress { .. } => {
                    // Progress events are fine, just skip in this test
                }
                ScanEvent::ScanComplete {
                    total_entries,
                    total_logical_bytes,
                    total_physical_bytes,
                    skipped,
                } => {
                    complete = Some((
                        total_entries,
                        total_logical_bytes,
                        total_physical_bytes,
                        skipped,
                    ));
                }
            }
        }

        // Should have: root dir, file1.txt, file2.txt, subdir, subdir/nested.txt
        assert!(
            entries.len() >= 4,
            "Expected at least 4 entries, got {}",
            entries.len()
        );

        let (_total_entries, total_logical_bytes, total_physical_bytes, skipped) =
            complete.expect("Should receive ScanComplete");
        // file1.txt=5 + file2.txt=6 + nested.txt=14 = 25
        assert_eq!(total_logical_bytes, 25);
        #[cfg(unix)]
        assert_eq!(total_physical_bytes, expected_physical_bytes);
        #[cfg(not(unix))]
        assert_eq!(total_physical_bytes, 25);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn test_scan_should_not_double_count_hard_linked_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::write(root.join("file1.txt"), "hello").unwrap();
        fs::hard_link(root.join("file1.txt"), root.join("file1-copy.txt")).unwrap();

        #[cfg(unix)]
        let expected_physical_bytes =
            fs::metadata(root.join("file1.txt")).unwrap().blocks() * STAT_BLOCK_SIZE_BYTES;

        let rx = scan(root);

        let mut complete = None;
        for event in rx {
            if let ScanEvent::ScanComplete {
                total_entries,
                total_logical_bytes,
                total_physical_bytes,
                skipped,
            } = event
            {
                complete = Some((
                    total_entries,
                    total_logical_bytes,
                    total_physical_bytes,
                    skipped,
                ));
            }
        }

        let (total_entries, total_logical_bytes, total_physical_bytes, skipped) =
            complete.expect("Should receive ScanComplete");
        assert_eq!(
            total_logical_bytes, 10,
            "logical bytes should still include both hard-linked paths"
        );
        assert_eq!(
            total_physical_bytes,
            {
                #[cfg(unix)]
                {
                    expected_physical_bytes
                }
                #[cfg(not(unix))]
                {
                    5
                }
            },
            "hard links should count once toward accounted physical size"
        );
        assert_eq!(
            total_entries, 3,
            "root dir and both paths should still be scanned"
        );
        assert_eq!(skipped, 0);
    }

    #[cfg(unix)]
    #[test]
    fn test_scan_should_use_allocated_blocks_for_physical_size() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = root.join("sparse.bin");

        File::create(&path).unwrap().set_len(8192).unwrap();

        let metadata = fs::metadata(&path).unwrap();
        let expected_physical_bytes = metadata.blocks() * 512;
        assert_ne!(
            expected_physical_bytes,
            metadata.len(),
            "test file must expose different logical and physical sizes"
        );

        let rx = scan(root);

        let mut scanned_sizes = None;
        for event in rx {
            if let ScanEvent::Entry {
                path: entry_path,
                sizes,
                ..
            } = event
            {
                if entry_path == path {
                    scanned_sizes = Some(sizes);
                }
            }
        }

        let scanned_sizes = scanned_sizes.expect("Should scan sparse file entry");
        assert_eq!(scanned_sizes.logical_bytes, metadata.len());
        assert_eq!(scanned_sizes.physical_bytes, expected_physical_bytes);
        assert_eq!(
            scanned_sizes.accounted_physical_bytes, expected_physical_bytes,
            "non-hard-linked file should fully count toward physical totals"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_scan_should_emit_hard_link_identity_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let original_path = root.join("file1.txt");
        let link_path = root.join("file1-copy.txt");

        fs::write(&original_path, "hello").unwrap();
        fs::hard_link(&original_path, &link_path).unwrap();

        let metadata = fs::metadata(&original_path).unwrap();
        let expected_identity = FileIdentity {
            dev: metadata.dev(),
            ino: metadata.ino(),
            nlink: metadata.nlink(),
        };

        let rx = scan(root);

        let mut original_identity = None;
        let mut link_identity = None;
        for event in rx {
            if let ScanEvent::Entry {
                path,
                file_identity,
                is_dir,
                ..
            } = event
            {
                if is_dir {
                    continue;
                }

                if path == original_path {
                    original_identity = file_identity;
                } else if path == link_path {
                    link_identity = file_identity;
                }
            }
        }

        assert_eq!(original_identity, Some(expected_identity));
        assert_eq!(link_identity, Some(expected_identity));
    }

    #[test]
    fn scan_profile_exclusion_should_skip_matching_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let keep_path = root.join("src").join("main.rs");
        let excluded_path = root.join("node_modules").join("pkg").join("index.js");

        fs::create_dir_all(keep_path.parent().unwrap()).unwrap();
        fs::create_dir_all(excluded_path.parent().unwrap()).unwrap();
        fs::write(&keep_path, "fn main() {}\n").unwrap();
        fs::write(&excluded_path, "module.exports = {};\n").unwrap();

        let profile = ScanProfile {
            name: "exclude-node-modules".to_string(),
            exclude: Some(Filter::single(FilterTest::PathGlob(
                "**/node_modules/**".to_string(),
            ))),
            mask: None,
            display_filter: None,
        };

        let rx = scan_with_profile(root, Some(profile));

        let mut seen_paths = Vec::new();
        for event in rx {
            if let ScanEvent::Entry { path, .. } = event {
                seen_paths.push(path);
            }
        }

        assert!(seen_paths.iter().any(|path| path == &keep_path));
        assert!(seen_paths
            .iter()
            .all(|path| !path.starts_with(root.join("node_modules"))));
    }

    #[test]
    fn excluded_paths_should_not_be_counted_in_scan_totals() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let included_path = root.join("src").join("main.rs");
        let excluded_path = root.join("node_modules").join("pkg").join("index.js");

        fs::create_dir_all(included_path.parent().unwrap()).unwrap();
        fs::create_dir_all(excluded_path.parent().unwrap()).unwrap();
        fs::write(&included_path, b"abcd").unwrap();
        fs::write(&excluded_path, b"0123456789").unwrap();

        let included_metadata = fs::metadata(&included_path).unwrap();

        let profile = ScanProfile {
            name: "exclude-node-modules".to_string(),
            exclude: Some(Filter::single(FilterTest::PathGlob(
                "**/node_modules/**".to_string(),
            ))),
            mask: None,
            display_filter: None,
        };

        let rx = scan_with_profile(root, Some(profile));

        let mut complete = None;
        for event in rx {
            if let ScanEvent::ScanComplete {
                total_entries,
                total_logical_bytes,
                total_physical_bytes,
                skipped,
            } = event
            {
                complete = Some((
                    total_entries,
                    total_logical_bytes,
                    total_physical_bytes,
                    skipped,
                ));
            }
        }

        let (total_entries, total_logical_bytes, total_physical_bytes, skipped) =
            complete.expect("Should receive ScanComplete");
        assert_eq!(total_entries, 3);
        assert_eq!(total_logical_bytes, included_metadata.len());
        #[cfg(unix)]
        assert_eq!(
            total_physical_bytes,
            included_metadata.blocks() * STAT_BLOCK_SIZE_BYTES
        );
        #[cfg(not(unix))]
        assert_eq!(total_physical_bytes, included_metadata.len());
        assert_eq!(skipped, 0);
    }
}
