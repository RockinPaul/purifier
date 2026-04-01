# GrandPerspective-Style Size And Scan Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Purifier behave much closer to GrandPerspective by defaulting to physical disk usage, exposing a logical/physical size toggle, accounting for hard links truthfully, adding GrandPerspective-style filters that can mask/filter/exclude paths, and keeping blocking scans responsive and truthful.

**Architecture:** Introduce a dedicated size-and-scan domain in `purifier-core` that computes logical bytes, physical bytes, hard-link identity, and filter matches before the TUI sees the data. Keep the current blocking scan UX, but move scan aggregation and progress coalescing into the scan worker so the TUI only renders progress snapshots and the final tree. Persist size mode and scan profiles in the TUI config so the UI, delete flow, and docs all speak the same size semantics.

**Tech Stack:** Rust workspace, `jwalk`, `crossbeam-channel`, `ratatui`, `crossterm`, macOS/Unix `MetadataExt`, `libc` for volume free-space sampling if needed.

---

## File Structure

**Create:**
- `crates/purifier-core/src/filters.rs` — GrandPerspective-style filter tests, filter trees, and evaluation for mask/filter/exclude behavior.
- `crates/purifier-core/src/size.rs` — `SizeMode`, size metrics, hard-link identity, and helpers for display/accounting semantics.

**Modify:**
- `crates/purifier-core/src/lib.rs` — export new size/filter modules and make deletion return truthful size deltas.
- `crates/purifier-core/src/scanner.rs` — compute logical/physical bytes, de-duplicate hard links, coalesce progress, apply exclusions, and emit richer scan results.
- `crates/purifier-core/src/types.rs` — replace the single `size` field/event payloads with explicit size data and metadata needed by the TUI.
- `crates/purifier-tui/src/app.rs` — persist selected size mode and active scan profile; rebuild flat entries using the selected size mode.
- `crates/purifier-tui/src/config.rs` — persist size mode, scan profiles, and last selected profile.
- `crates/purifier-tui/src/input.rs` — add size-mode toggle and filter/profile controls; keep scan cancel/quit responsive.
- `crates/purifier-tui/src/main.rs` — rework scan event consumption to use coalesced progress + final result, not per-entry UI-thread tree work.
- `crates/purifier-tui/src/ui/settings_modal.rs` — show persisted size mode/profile settings.
- `crates/purifier-tui/src/ui/status_bar.rs` — show active size mode and active filter/profile during scan and after completion.
- `crates/purifier-tui/src/ui/tree_view.rs` — render both logical/physical info in the info pane and sort/display using the active size mode.
- `README.md` — document physical vs logical size, hard-link handling, filter semantics, and blocking scan behavior.
- `PROGRESS.md` — update current status after each major milestone lands.
- `AGENTS.md` — update repo guidance if size semantics or scan behavior expectations change.

**Test Files:**
- `crates/purifier-core/src/size.rs` unit tests
- `crates/purifier-core/src/filters.rs` unit tests
- `crates/purifier-core/src/scanner.rs` unit tests
- `crates/purifier-tui/src/app.rs` unit tests
- `crates/purifier-tui/src/main.rs` unit tests
- `crates/purifier-tui/src/input.rs` unit tests
- `crates/purifier-tui/src/ui/tree_view.rs` unit tests
- `crates/purifier-tui/src/ui/status_bar.rs` unit tests

## Product Decisions Locked In

- Default size mode is `Physical`.
- A user-visible toggle switches between `Physical` and `Logical` size.
- Blocking scan remains the default UX; no progressive browsing during scan in this plan.
- Scan-time filters follow the GrandPerspective model and can be used to:
  - mask files in the view
  - filter files from the view
  - exclude files/folders during scan
- Freed-space reporting should prefer observed on-disk free-space delta over naive `metadata.len()` sums.
- APFS clone/shared-block exactness is not a requirement; the app must state that physical size is best-effort and clone sharing is approximate.

## Data Model Decisions Locked In

- New core size types live in `crates/purifier-core/src/size.rs`.
- `FileEntry` and `ScanEvent` carry explicit size metrics instead of a single `size` number.
- Directory totals are computed through `total_size(mode)` so all views stay consistent.
- Hard-link identity is tracked on Unix using `(dev, ino, nlink)`.
- Filter definitions live in `purifier-core`, not in the TUI, so the same logic can drive scan exclusion and view filtering.

## Task 1: Introduce A First-Class Size Model

**Files:**
- Create: `crates/purifier-core/src/size.rs`
- Modify: `crates/purifier-core/src/lib.rs`
- Modify: `crates/purifier-core/src/types.rs`
- Test: `crates/purifier-core/src/size.rs`

- [ ] **Step 1: Write the failing tests for size semantics**

```rust
#[cfg(test)]
mod tests {
    use super::{EntrySizes, FileIdentity, SizeMode};

    #[test]
    fn physical_mode_should_use_accounted_physical_bytes() {
        let sizes = EntrySizes {
            logical_bytes: 100,
            physical_bytes: 4096,
            accounted_physical_bytes: 0,
        };

        assert_eq!(sizes.display_bytes(SizeMode::Physical), 4096);
    }

    #[test]
    fn logical_mode_should_use_logical_bytes() {
        let sizes = EntrySizes {
            logical_bytes: 100,
            physical_bytes: 4096,
            accounted_physical_bytes: 4096,
        };

        assert_eq!(sizes.display_bytes(SizeMode::Logical), 100);
    }

    #[test]
    fn hard_link_identity_should_round_trip() {
        let identity = FileIdentity {
            dev: 7,
            ino: 42,
            nlink: 3,
        };

        assert_eq!(identity.dev, 7);
        assert_eq!(identity.ino, 42);
        assert_eq!(identity.nlink, 3);
    }
}
```

- [ ] **Step 2: Run the targeted tests to verify they fail**

Run: `cargo test -p purifier-core physical_mode_should_use_accounted_physical_bytes && cargo test -p purifier-core logical_mode_should_use_logical_bytes && cargo test -p purifier-core hard_link_identity_should_round_trip`
Expected: FAIL because `EntrySizes`, `FileIdentity`, and `SizeMode` do not exist yet.

- [ ] **Step 3: Implement the new size types and thread them into `FileEntry` and `ScanEvent`**

```rust
// crates/purifier-core/src/size.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SizeMode {
    Physical,
    Logical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EntrySizes {
    pub logical_bytes: u64,
    pub physical_bytes: u64,
    pub accounted_physical_bytes: u64,
}

impl EntrySizes {
    pub fn display_bytes(self, mode: SizeMode) -> u64 {
        match mode {
            SizeMode::Physical => self.physical_bytes,
            SizeMode::Logical => self.logical_bytes,
        }
    }

    pub fn accounted_total_bytes(self, mode: SizeMode) -> u64 {
        match mode {
            SizeMode::Physical => self.accounted_physical_bytes,
            SizeMode::Logical => self.logical_bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
}
```

```rust
// crates/purifier-core/src/types.rs
use crate::size::{EntrySizes, FileIdentity, SizeMode};

pub struct FileEntry {
    pub path: PathBuf,
    pub sizes: EntrySizes,
    pub file_identity: Option<FileIdentity>,
    pub is_dir: bool,
    // existing fields unchanged
}

impl FileEntry {
    pub fn total_size(&self, mode: SizeMode) -> u64 {
        if self.children.is_empty() {
            self.sizes.display_bytes(mode)
        } else {
            self.children.iter().map(|child| child.total_size(mode)).sum()
        }
    }
}

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
```

- [ ] **Step 4: Run the targeted core tests**

Run: `cargo test -p purifier-core size::tests`
Expected: PASS

- [ ] **Step 5: Run the broader core test suite**

Run: `cargo test -p purifier-core`
Expected: PASS with existing classifier/rules/scanner tests updated to compile against the new size model.

- [ ] **Step 6: Commit**

```bash
git add crates/purifier-core/src/size.rs crates/purifier-core/src/lib.rs crates/purifier-core/src/types.rs
git commit -m "Add explicit logical and physical size model"
```

### Task 2: Make The Scanner Compute Physical Size And Hard-Link Metadata

**Files:**
- Modify: `crates/purifier-core/src/scanner.rs`
- Modify: `crates/purifier-core/src/types.rs`
- Test: `crates/purifier-core/src/scanner.rs`

- [ ] **Step 1: Write failing scanner tests for physical size and hard-link metadata**

```rust
#[test]
fn scan_should_report_allocated_blocks_as_physical_size() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("tiny.txt"), b"x").unwrap();

    let rx = scan(dir.path());
    let complete = rx.into_iter().find_map(|event| match event {
        ScanEvent::ScanComplete { total_physical_bytes, .. } => Some(total_physical_bytes),
        _ => None,
    }).unwrap();

    assert!(complete >= 512, "physical size should use allocated blocks");
}

#[test]
fn scan_should_emit_file_identity_for_hard_links() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("file.txt");
    let linked = dir.path().join("file-copy.txt");
    std::fs::write(&original, b"hello").unwrap();
    std::fs::hard_link(&original, &linked).unwrap();

    let rx = scan(dir.path());
    let entries: Vec<_> = rx.into_iter().filter_map(|event| match event {
        ScanEvent::Entry { file_identity, .. } => file_identity,
        _ => None,
    }).collect();

    assert!(entries.iter().any(|identity| identity.nlink > 1));
}
```

- [ ] **Step 2: Run the targeted scanner tests to verify they fail**

Run: `cargo test -p purifier-core scan_should_report_allocated_blocks_as_physical_size && cargo test -p purifier-core scan_should_emit_file_identity_for_hard_links`
Expected: FAIL because scan events do not yet carry those fields/semantics.

- [ ] **Step 3: Implement macOS/Unix physical byte calculation and file identity tracking**

```rust
#[cfg(unix)]
fn physical_bytes(metadata: &std::fs::Metadata) -> u64 {
    metadata.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn physical_bytes(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

#[cfg(unix)]
fn file_identity(metadata: &std::fs::Metadata) -> Option<FileIdentity> {
    Some(FileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        nlink: metadata.nlink(),
    })
}

#[cfg(not(unix))]
fn file_identity(_: &std::fs::Metadata) -> Option<FileIdentity> {
    None
}
```

```rust
let sizes = EntrySizes {
    logical_bytes: metadata.len(),
    physical_bytes: physical_bytes(&metadata),
    accounted_physical_bytes: accounted_physical_bytes(&metadata, &mut seen_file_ids),
};
```

- [ ] **Step 4: Make progress totals use accounted physical bytes and logical bytes separately**

Run: `cargo test -p purifier-core scanner::tests`
Expected: PASS, including the existing hard-link regression updated to assert `total_physical_bytes` semantics.

- [ ] **Step 5: Commit**

```bash
git add crates/purifier-core/src/scanner.rs crates/purifier-core/src/types.rs
git commit -m "Teach scanner physical size and hard-link metadata"
```

### Task 3: Make Delete And Freed-Space Reporting Truthful

**Files:**
- Modify: `crates/purifier-core/src/lib.rs`
- Modify: `crates/purifier-tui/src/input.rs`
- Modify: `crates/purifier-tui/src/app.rs`
- Modify: `crates/purifier-tui/src/ui/tree_view.rs`
- Test: `crates/purifier-core/src/lib.rs`
- Test: `crates/purifier-tui/src/input.rs`

- [ ] **Step 1: Write failing tests for deletion accounting**

```rust
#[test]
fn delete_entry_should_return_physical_delta_when_available() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("delete-me.bin");
    std::fs::write(&file, vec![0u8; 8192]).unwrap();

    let result = delete_entry(&file).unwrap();

    assert!(result.physical_bytes_freed > 0);
    assert_eq!(result.logical_bytes_removed, 8192);
}
```

- [ ] **Step 2: Run the targeted delete tests to verify they fail**

Run: `cargo test -p purifier-core delete_entry_should_return_physical_delta_when_available`
Expected: FAIL because `delete_entry` currently returns a single `u64`.

- [ ] **Step 3: Introduce a structured delete result and free-space delta helper**

```rust
pub struct DeleteOutcome {
    pub logical_bytes_removed: u64,
    pub physical_bytes_estimated: u64,
    pub physical_bytes_freed: u64,
}

pub fn delete_entry(path: &Path) -> Result<DeleteOutcome, std::io::Error> {
    let before = volume_free_bytes(path).ok();
    let estimated = estimate_delete_sizes(path)?;

    if std::fs::metadata(path)?.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }

    let after = volume_free_bytes(path.parent().unwrap_or(path)).ok();
    let physical_bytes_freed = match (before, after) {
        (Some(before), Some(after)) if after >= before => after - before,
        _ => estimated.accounted_physical_bytes,
    };

    Ok(DeleteOutcome {
        logical_bytes_removed: estimated.logical_bytes,
        physical_bytes_estimated: estimated.accounted_physical_bytes,
        physical_bytes_freed,
    })
}
```

- [ ] **Step 4: Update TUI delete flow to surface both logical and physical values**

Run: `cargo test -p purifier-tui confirm_delete_should_keep_entry_and_record_error_when_delete_fails`
Expected: PASS after updating the delete flow and UI text to use `DeleteOutcome`.

- [ ] **Step 5: Run full verification for core + TUI delete paths**

Run: `cargo test -p purifier-core && cargo test -p purifier-tui`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/purifier-core/src/lib.rs crates/purifier-tui/src/input.rs crates/purifier-tui/src/app.rs crates/purifier-tui/src/ui/tree_view.rs
git commit -m "Make freed-space reporting match physical disk usage"
```

### Task 4: Add GrandPerspective-Style Filters And Scan Profiles

**Files:**
- Create: `crates/purifier-core/src/filters.rs`
- Modify: `crates/purifier-core/src/lib.rs`
- Modify: `crates/purifier-core/src/scanner.rs`
- Modify: `crates/purifier-tui/src/config.rs`
- Modify: `crates/purifier-tui/src/main.rs`
- Test: `crates/purifier-core/src/filters.rs`
- Test: `crates/purifier-core/src/scanner.rs`
- Test: `crates/purifier-tui/src/config.rs`

- [ ] **Step 1: Write failing filter evaluation tests**

```rust
#[test]
fn filter_should_match_by_path_glob_and_hard_link_status() {
    let filter = Filter::all([
        FilterTest::PathGlob("**/node_modules/**".to_string()),
        FilterTest::HardLinkStatus(HardLinkStatus::Any),
    ]);

    let meta = FilterEntryMeta {
        path: PathBuf::from("/tmp/project/node_modules/pkg/index.js"),
        logical_bytes: 100,
        physical_bytes: 4096,
        is_dir: false,
        is_package: false,
        hard_link_status: HardLinkStatus::Any,
    };

    assert!(filter.matches(&meta));
}

#[test]
fn scan_profile_should_exclude_matching_paths() {
    let profile = ScanProfile {
        name: "exclude-node-modules".to_string(),
        exclude: Some(Filter::single(FilterTest::PathGlob("**/node_modules/**".to_string()))),
        mask: None,
        display_filter: None,
    };

    assert!(profile.should_exclude(Path::new("/tmp/app/node_modules/react/index.js")));
}
```

- [ ] **Step 2: Run the targeted filter tests to verify they fail**

Run: `cargo test -p purifier-core filter_should_match_by_path_glob_and_hard_link_status && cargo test -p purifier-core scan_profile_should_exclude_matching_paths`
Expected: FAIL because filter types do not exist.

- [ ] **Step 3: Implement filter tests and scan profiles**

```rust
pub enum FilterTest {
    NameContains(String),
    PathGlob(String),
    SizeAtLeast(u64),
    SizeAtMost(u64),
    FileType(FileTypeMatch),
    HardLinkStatus(HardLinkStatus),
    PackageStatus(PackageStatus),
}

pub enum Filter {
    Single(FilterTest),
    All(Vec<Filter>),
    Any(Vec<Filter>),
    Not(Box<Filter>),
}

pub struct ScanProfile {
    pub name: String,
    pub exclude: Option<Filter>,
    pub mask: Option<Filter>,
    pub display_filter: Option<Filter>,
}
```

- [ ] **Step 4: Apply `exclude` filters inside the scanner walk before accounting**

Run: `cargo test -p purifier-core scanner::tests`
Expected: PASS with new tests proving excluded paths are neither walked into the final tree nor counted in totals.

- [ ] **Step 5: Persist scan profiles in app config**

Run: `cargo test -p purifier-tui config::tests`
Expected: PASS with new round-trip tests for `size_mode` and `scan_profiles` persistence.

- [ ] **Step 6: Commit**

```bash
git add crates/purifier-core/src/filters.rs crates/purifier-core/src/lib.rs crates/purifier-core/src/scanner.rs crates/purifier-tui/src/config.rs crates/purifier-tui/src/main.rs
git commit -m "Add GrandPerspective-style filters and scan profiles"
```

### Task 5: Refactor The Scan Pipeline For Responsive Blocking Scans

**Files:**
- Modify: `crates/purifier-core/src/scanner.rs`
- Modify: `crates/purifier-tui/src/main.rs`
- Modify: `crates/purifier-tui/src/app.rs`
- Modify: `crates/purifier-tui/src/input.rs`
- Modify: `crates/purifier-tui/src/ui/status_bar.rs`
- Modify: `crates/purifier-tui/src/ui/tree_view.rs`
- Test: `crates/purifier-core/src/scanner.rs`
- Test: `crates/purifier-tui/src/main.rs`
- Test: `crates/purifier-tui/src/input.rs`

- [ ] **Step 1: Write failing responsiveness tests**

```rust
#[test]
fn start_scan_should_return_progress_snapshots_without_per_entry_tree_rebuilds() {
    let mut app = App::new(Some(PathBuf::from("/scan")), false, AppConfig::default());
    app.start_scan_with_path(PathBuf::from("/scan"));

    let snapshot = ScanProgressSnapshot {
        entries_scanned: 10_000,
        logical_bytes_found: 100,
        physical_bytes_found: 4096,
        current_path: "/scan/tmp".to_string(),
    };

    apply_scan_progress_snapshot(&mut app, &snapshot);

    assert!(app.flat_entries.is_empty(), "blocking scan should not rebuild hidden rows");
    assert_eq!(app.bytes_found, 4096);
}

#[test]
fn scanning_state_should_allow_quit_without_waiting_for_scan_completion() {
    let mut app = App::new(Some(PathBuf::from("/scan")), false, AppConfig::default());
    app.scan_status = ScanStatus::Scanning;

    handle_key(&mut app, KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));

    assert!(app.should_quit);
}
```

- [ ] **Step 2: Run the targeted TUI tests to verify the current pipeline fails the new expectations**

Run: `cargo test -p purifier-tui start_scan_should_return_progress_snapshots_without_per_entry_tree_rebuilds && cargo test -p purifier-tui scanning_state_should_allow_quit_without_waiting_for_scan_completion`
Expected: FAIL because progress snapshots do not exist yet.

- [ ] **Step 3: Replace flood-style `Entry` rendering work with coalesced progress + final scan result**

```rust
pub enum ScanEvent {
    Progress(ScanProgressSnapshot),
    Complete(ScanResult),
    Cancelled,
}

pub struct ScanProgressSnapshot {
    pub entries_scanned: u64,
    pub logical_bytes_found: u64,
    pub physical_bytes_found: u64,
    pub current_path: String,
}

pub struct ScanResult {
    pub entries: Vec<FileEntry>,
    pub total_entries: u64,
    pub total_logical_bytes: u64,
    pub total_physical_bytes: u64,
    pub skipped: u64,
}
```

- [ ] **Step 4: Add a scan control channel for cancellation**

Run: `cargo test -p purifier-tui scanning_state_should_ignore_main_list_navigation_keys && cargo test -p purifier-tui mouse_wheel_should_not_move_selection_while_scanning`
Expected: PASS while scan progress remains blocking and `q`/`Esc` stay responsive.

- [ ] **Step 5: Run the full TUI suite and clippy**

Run: `cargo test -p purifier-tui && cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/purifier-core/src/scanner.rs crates/purifier-tui/src/main.rs crates/purifier-tui/src/app.rs crates/purifier-tui/src/input.rs crates/purifier-tui/src/ui/status_bar.rs crates/purifier-tui/src/ui/tree_view.rs
git commit -m "Refactor blocking scans to use responsive progress snapshots"
```

### Task 6: Add UI Controls For Size Mode And Scan Profiles

**Files:**
- Modify: `crates/purifier-tui/src/app.rs`
- Modify: `crates/purifier-tui/src/config.rs`
- Modify: `crates/purifier-tui/src/input.rs`
- Modify: `crates/purifier-tui/src/ui/settings_modal.rs`
- Modify: `crates/purifier-tui/src/ui/status_bar.rs`
- Modify: `crates/purifier-tui/src/ui/tree_view.rs`
- Test: `crates/purifier-tui/src/app.rs`
- Test: `crates/purifier-tui/src/input.rs`
- Test: `crates/purifier-tui/src/ui/status_bar.rs`
- Test: `crates/purifier-tui/src/ui/tree_view.rs`

- [ ] **Step 1: Write failing UI tests for size-mode display and scan-profile status**

```rust
#[test]
fn rebuild_flat_entries_should_sort_by_selected_size_mode() {
    let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
    app.size_mode = SizeMode::Physical;
    app.entries = vec![
        FileEntry::new_with_sizes(PathBuf::from("/logical-large"), 10_000, 4096, false, None),
        FileEntry::new_with_sizes(PathBuf::from("/physical-large"), 100, 8192, false, None),
    ];

    app.rebuild_flat_entries();

    assert_eq!(app.flat_entries[0].path, PathBuf::from("/physical-large"));
}

#[test]
fn status_bar_should_show_active_size_mode_and_profile() {
    let mut app = App::new(Some(PathBuf::from("/")), false, AppConfig::default());
    app.size_mode = SizeMode::Physical;
    app.active_scan_profile = Some("Fast developer scan".to_string());

    let line = build_status_line(&app);

    assert!(line.contains("Size: Physical"));
    assert!(line.contains("Profile: Fast developer scan"));
}
```

- [ ] **Step 2: Run the targeted UI tests to verify they fail**

Run: `cargo test -p purifier-tui rebuild_flat_entries_should_sort_by_selected_size_mode && cargo test -p purifier-tui status_bar_should_show_active_size_mode_and_profile`
Expected: FAIL because `size_mode` and `active_scan_profile` do not exist yet.

- [ ] **Step 3: Persist and render size mode/profile state**

```rust
pub struct UiPreferences {
    pub default_view: View,
    pub last_scan_path: Option<PathBuf>,
    pub size_mode: SizeMode,
    pub active_scan_profile: Option<String>,
}
```

```rust
pub struct FlatEntry {
    pub path: PathBuf,
    pub size: u64,
    pub logical_size: u64,
    pub physical_size: u64,
    // existing fields unchanged
}
```

- [ ] **Step 4: Add input controls and settings-modal controls**

Run: `cargo test -p purifier-tui input:: && cargo test -p purifier-tui ui::status_bar:: && cargo test -p purifier-tui ui::tree_view::`
Expected: PASS with tests proving that size-mode toggles change sorting/display and settings persist.

- [ ] **Step 5: Commit**

```bash
git add crates/purifier-tui/src/app.rs crates/purifier-tui/src/config.rs crates/purifier-tui/src/input.rs crates/purifier-tui/src/ui/settings_modal.rs crates/purifier-tui/src/ui/status_bar.rs crates/purifier-tui/src/ui/tree_view.rs
git commit -m "Expose size mode and scan profiles in the TUI"
```

### Task 7: Add macOS Package Awareness And Curated Built-In Profiles

**Files:**
- Modify: `crates/purifier-core/src/filters.rs`
- Modify: `crates/purifier-core/src/scanner.rs`
- Modify: `crates/purifier-tui/src/config.rs`
- Modify: `README.md`
- Test: `crates/purifier-core/src/filters.rs`
- Test: `crates/purifier-core/src/scanner.rs`

- [ ] **Step 1: Write failing tests for package status and a built-in profile**

```rust
#[test]
fn package_status_should_match_common_macos_bundles() {
    assert!(is_package_path(Path::new("/Applications/Foo.app")));
    assert!(is_package_path(Path::new("/Library/Frameworks/Bar.framework")));
    assert!(!is_package_path(Path::new("/tmp/plain-dir")));
}

#[test]
fn built_in_profile_should_exclude_node_modules_when_requested() {
    let profile = built_in_scan_profiles()
        .into_iter()
        .find(|profile| profile.name == "Fast developer scan")
        .unwrap();

    assert!(profile.should_exclude(Path::new("/tmp/app/node_modules/react/index.js")));
}
```

- [ ] **Step 2: Run the targeted package/profile tests to verify they fail**

Run: `cargo test -p purifier-core package_status_should_match_common_macos_bundles && cargo test -p purifier-core built_in_profile_should_exclude_node_modules_when_requested`
Expected: FAIL because package helpers and built-in profiles do not exist yet.

- [ ] **Step 3: Add package detection and built-in profiles**

```rust
fn is_package_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("app" | "bundle" | "framework" | "plugin" | "kext" | "pkg")
    )
}

fn built_in_scan_profiles() -> Vec<ScanProfile> {
    vec![
        ScanProfile::new("Full scan"),
        ScanProfile::exclude_globs(
            "Fast developer scan",
            vec!["**/node_modules/**", "**/target/**", "**/DerivedData/**"],
        ),
    ]
}
```

- [ ] **Step 4: Update README to explain package status, size semantics, and filter modes**

Run: `cargo test -p purifier-core && cargo test -p purifier-tui`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/purifier-core/src/filters.rs crates/purifier-core/src/scanner.rs crates/purifier-tui/src/config.rs README.md
git commit -m "Add package awareness and built-in scan profiles"
```

### Task 8: Docs And Repo Guidance Synchronization

**Files:**
- Modify: `README.md`
- Modify: `PROGRESS.md`
- Modify: `AGENTS.md`

- [ ] **Step 1: Write the doc changes reflecting the final product behavior**

```md
- Default size mode is physical disk usage.
- Users can switch between physical and logical size.
- Hard-linked files are counted once for physical totals.
- Scan profiles can mask, filter, or exclude paths.
- Blocking scans remain responsive and cancellable.
- APFS clone/shared-block accounting is approximate.
```

- [ ] **Step 2: Verify docs match actual implemented behavior**

Run: `rg -n "progressive UI|size|filter|hard-link|physical|logical" README.md PROGRESS.md AGENTS.md`
Expected: docs use the new truthful wording and do not claim hidden live browsing during scans.

- [ ] **Step 3: Run final verification**

Run: `cargo test -p purifier-core && cargo test -p purifier-tui && cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add README.md PROGRESS.md AGENTS.md
git commit -m "Document GrandPerspective-style size and filter behavior"
```

## Notes On Semantics

- `Logical` size means file content length, comparable to Finder's standard file size.
- `Physical` size means allocated bytes from filesystem blocks, not exact unique APFS clone ownership.
- `accounted_physical_bytes` is used for totals so hard-linked storage is counted once.
- Row display in `Physical` mode should show `physical_bytes`, not `accounted_physical_bytes`, because a visible path still needs a stable per-path value. Totals and delete summaries should additionally explain when storage is shared.
- For hard links, the info pane and delete confirm must state that deleting one link may free less than the row's displayed physical size.

## Final Verification Checklist

- [ ] `cargo test -p purifier-core`
- [ ] `cargo test -p purifier-tui`
- [ ] `cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings`
- [ ] README describes logical vs physical size truthfully
- [ ] README describes mask/filter/exclude semantics truthfully
- [ ] Delete confirmation warns when storage is hard-link shared
- [ ] Blocking scan remains responsive to `q` / `Esc`
- [ ] Scan progress shows logical and physical totals clearly
