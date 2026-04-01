pub mod classifier;
pub mod filters;
pub mod llm;
pub mod provider;
pub mod rules;
pub mod scanner;
pub mod size;
pub mod types;

pub use filters::{
    built_in_scan_profiles, FileTypeMatch, Filter, FilterEntryMeta, FilterTest, HardLinkStatus,
    PackageStatus, ScanProfile,
};
pub use provider::{
    LlmClient, LlmError, ProviderKind, ProviderSettings, ProviderSettingsMap,
    ResolvedProviderConfig,
};
pub use size::{EntrySizes, FileIdentity, SizeMode};
pub use types::{Category, FileEntry, SafetyLevel, ScanEvent};

use std::io;
use std::path::Path;

#[cfg(unix)]
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeleteOutcome {
    pub logical_bytes_removed: u64,
    pub physical_bytes_estimated: u64,
    pub physical_bytes_freed: u64,
    pub entries_removed: u64,
}

pub fn delete_entry(path: &Path) -> Result<DeleteOutcome, std::io::Error> {
    let metadata = std::fs::symlink_metadata(path)?;
    let sample_path = volume_sample_path(path);
    let free_bytes_before = volume_free_bytes(sample_path).ok();
    let account = if metadata.is_dir() {
        dir_delete_account(path)?
    } else {
        #[cfg(unix)]
        {
            DeleteAccount {
                sizes: file_sizes(&metadata, &mut HashSet::new(), 1),
                entries_removed: 1,
            }
        }

        #[cfg(not(unix))]
        {
            DeleteAccount {
                sizes: file_sizes(&metadata, &mut ()),
                entries_removed: 1,
            }
        }
    };

    if metadata.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }

    let free_bytes_after = volume_free_bytes(sample_path).ok();

    Ok(DeleteOutcome {
        logical_bytes_removed: account.sizes.logical_bytes,
        physical_bytes_estimated: account.sizes.accounted_physical_bytes,
        physical_bytes_freed: observed_physical_bytes_freed(free_bytes_before, free_bytes_after),
        entries_removed: account.entries_removed,
    })
}

fn observed_physical_bytes_freed(before: Option<u64>, after: Option<u64>) -> u64 {
    match (before, after) {
        (Some(before), Some(after)) if after >= before => after - before,
        _ => 0,
    }
}

fn volume_sample_path(path: &Path) -> &Path {
    path.parent().unwrap_or(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct DeleteAccount {
    sizes: EntrySizes,
    entries_removed: u64,
}

#[cfg(unix)]
fn file_sizes(
    metadata: &std::fs::Metadata,
    seen_file_ids: &mut HashSet<(u64, u64)>,
    target_link_count: u64,
) -> EntrySizes {
    use std::os::unix::fs::MetadataExt;

    const STAT_BLOCK_SIZE_BYTES: u64 = 512;
    let logical_bytes = metadata.len();
    let physical_bytes = metadata.blocks() * STAT_BLOCK_SIZE_BYTES;
    let accounted_physical_bytes = if metadata.nlink() > 1 {
        let file_id = (metadata.dev(), metadata.ino());
        if target_link_count == metadata.nlink() && seen_file_ids.insert(file_id) {
            physical_bytes
        } else {
            0
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
fn file_sizes(metadata: &std::fs::Metadata, _seen_file_ids: &mut ()) -> EntrySizes {
    let logical_bytes = metadata.len();
    EntrySizes {
        logical_bytes,
        physical_bytes: logical_bytes,
        accounted_physical_bytes: logical_bytes,
    }
}

#[cfg(unix)]
fn dir_delete_account(path: &Path) -> Result<DeleteAccount, io::Error> {
    let mut link_counts = HashMap::new();
    collect_target_link_counts(path, &mut link_counts)?;
    dir_delete_account_with_seen(path, &mut HashSet::new(), &link_counts)
}

#[cfg(unix)]
fn dir_delete_account_with_seen(
    path: &Path,
    seen_file_ids: &mut HashSet<(u64, u64)>,
    target_link_counts: &HashMap<(u64, u64), u64>,
) -> Result<DeleteAccount, io::Error> {
    let mut total = DeleteAccount {
        sizes: EntrySizes::default(),
        entries_removed: 1,
    };

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        let child_sizes = if metadata.is_dir() {
            dir_delete_account_with_seen(&entry.path(), seen_file_ids, target_link_counts)?
        } else {
            let target_link_count = file_link_count_in_target(&metadata, target_link_counts);
            DeleteAccount {
                sizes: file_sizes(&metadata, seen_file_ids, target_link_count),
                entries_removed: 1,
            }
        };

        total.sizes.logical_bytes += child_sizes.sizes.logical_bytes;
        total.sizes.physical_bytes += child_sizes.sizes.physical_bytes;
        total.sizes.accounted_physical_bytes += child_sizes.sizes.accounted_physical_bytes;
        total.entries_removed += child_sizes.entries_removed;
    }

    Ok(total)
}

#[cfg(not(unix))]
fn dir_delete_account(path: &Path) -> Result<DeleteAccount, io::Error> {
    let mut total = DeleteAccount {
        sizes: EntrySizes::default(),
        entries_removed: 1,
    };

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        let child_sizes = if metadata.is_dir() {
            dir_delete_account(&entry.path())?
        } else {
            DeleteAccount {
                sizes: file_sizes(&metadata, &mut ()),
                entries_removed: 1,
            }
        };

        total.sizes.logical_bytes += child_sizes.sizes.logical_bytes;
        total.sizes.physical_bytes += child_sizes.sizes.physical_bytes;
        total.sizes.accounted_physical_bytes += child_sizes.sizes.accounted_physical_bytes;
        total.entries_removed += child_sizes.entries_removed;
    }

    Ok(total)
}

#[cfg(unix)]
fn collect_target_link_counts(
    path: &Path,
    counts: &mut HashMap<(u64, u64), u64>,
) -> Result<(), io::Error> {
    use std::os::unix::fs::MetadataExt;

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            collect_target_link_counts(&entry.path(), counts)?;
            continue;
        }

        if metadata.file_type().is_symlink() || metadata.nlink() <= 1 {
            continue;
        }

        let file_id = (metadata.dev(), metadata.ino());
        *counts.entry(file_id).or_insert(0) += 1;
    }

    Ok(())
}

#[cfg(unix)]
fn file_link_count_in_target(
    metadata: &std::fs::Metadata,
    target_link_counts: &HashMap<(u64, u64), u64>,
) -> u64 {
    use std::os::unix::fs::MetadataExt;

    if metadata.file_type().is_symlink() || metadata.nlink() <= 1 {
        1
    } else {
        target_link_counts
            .get(&(metadata.dev(), metadata.ino()))
            .copied()
            .unwrap_or(1)
    }
}

#[cfg(unix)]
fn volume_free_bytes(path: &Path) -> Result<u64, io::Error> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL byte"))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let result = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    let stat = unsafe { stat.assume_init() };
    Ok(u64::from(stat.f_bavail).saturating_mul(stat.f_frsize))
}

#[cfg(not(unix))]
fn volume_free_bytes(_path: &Path) -> Result<u64, io::Error> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "volume free-space sampling is unavailable on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::{delete_entry, observed_physical_bytes_freed};

    #[test]
    fn delete_entry_should_return_structured_sizes() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let file = dir.path().join("delete-me.bin");
        std::fs::write(&file, vec![0_u8; 8192]).expect("test file should be written");

        let result = delete_entry(&file).expect("delete should succeed");

        assert_eq!(result.logical_bytes_removed, 8192);
        assert!(
            result.physical_bytes_estimated > 0,
            "delete outcome should carry a physical estimate"
        );
        assert_eq!(result.entries_removed, 1);
    }

    #[test]
    fn delete_entry_should_leave_observed_freed_bytes_at_zero_when_sampling_is_unavailable() {
        assert_eq!(observed_physical_bytes_freed(None, Some(10)), 0);
        assert_eq!(observed_physical_bytes_freed(Some(10), None), 0);
        assert_eq!(observed_physical_bytes_freed(Some(10), Some(8)), 0);
    }

    #[test]
    fn delete_entry_should_report_removed_entry_count_for_directory() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let root = dir.path().join("delete-dir");
        std::fs::create_dir(&root).expect("directory should be created");
        std::fs::write(root.join("a.bin"), vec![0_u8; 8]).expect("file should be written");
        std::fs::create_dir(root.join("nested")).expect("nested directory should be created");
        std::fs::write(root.join("nested").join("b.bin"), vec![0_u8; 8])
            .expect("nested file should be written");

        let result = delete_entry(&root).expect("delete should succeed");

        assert_eq!(result.entries_removed, 4);
    }

    #[cfg(unix)]
    #[test]
    fn delete_entry_should_not_follow_symlinked_directories() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let external = dir.path().join("external-target");
        std::fs::create_dir(&external).expect("external directory should be created");
        std::fs::write(external.join("kept.bin"), vec![0_u8; 8]).expect("external file should be written");

        let root = dir.path().join("delete-dir");
        std::fs::create_dir(&root).expect("directory should be created");
        std::os::unix::fs::symlink(&external, root.join("linked-dir"))
            .expect("symlink should be created");

        let result = delete_entry(&root).expect("delete should succeed");

        assert_eq!(result.entries_removed, 2);
        assert!(external.exists(), "external target should not be deleted");
        assert!(external.join("kept.bin").exists(), "external contents should remain");
    }

    #[cfg(unix)]
    #[test]
    fn delete_entry_should_treat_symlinked_directory_path_as_single_entry() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let external = dir.path().join("external-target");
        std::fs::create_dir(&external).expect("external directory should be created");
        std::fs::write(external.join("kept.bin"), vec![0_u8; 8]).expect("external file should be written");

        let link_path = dir.path().join("linked-dir");
        std::os::unix::fs::symlink(&external, &link_path).expect("symlink should be created");

        let result = delete_entry(&link_path).expect("delete should succeed");

        assert_eq!(result.entries_removed, 1);
        assert!(external.exists(), "symlink target should remain");
        assert!(external.join("kept.bin").exists(), "target contents should remain");
        assert!(!link_path.exists(), "symlink itself should be removed");
    }

    #[cfg(unix)]
    #[test]
    fn delete_entry_should_not_estimate_physical_free_when_other_hard_links_survive() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let original = dir.path().join("file.txt");
        let linked = dir.path().join("file-copy.txt");
        std::fs::write(&original, b"hello").expect("file should be written");
        std::fs::hard_link(&original, &linked).expect("hard link should be created");

        let result = delete_entry(&original).expect("delete should succeed");

        assert_eq!(result.logical_bytes_removed, 5);
        assert_eq!(result.physical_bytes_estimated, 0);
        assert!(linked.exists(), "surviving hard link should remain");
    }
}
