# Miller Columns TUI Rewrite Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the current tree-browser TUI with a Miller Columns (Finder-style) three-pane interface. This eliminates the full-tree-rebuild performance bottleneck by design — each interaction renders a single directory listing instead of flattening the entire tree. The current 4-tab view system is replaced by a rich preview pane that shows type, age, and safety breakdowns contextually. Deletion supports both quick single-item confirm and batch mark-and-sweep.

**Motivation:** The current TUI rebuilds the entire flat entry list on every expand/collapse toggle. This causes noticeable lag on large filesystems (100k+ entries). The experimental worktree (`interactive-view-model`) improved this partially but still lacks a retained indexed model. Rather than finishing that incremental fix, this plan changes the interaction paradigm so the problem cannot exist — Miller Columns never "expand" a tree, they navigate into directories one at a time.

**Architecture:** Rewrite `purifier-tui` around a column-stack navigation model. Each visible column is a sorted list of one directory's children. The preview (right) pane computes analytics lazily for the selected item. `purifier-core` is **unchanged** — all scanner, classifier, rules, LLM, size, and filter code stays as-is. Event loop improvements from the `interactive-view-model` worktree (input-before-draw, capped per-frame processing, coalesced progress) are carried forward into the new main loop.

**Tech Stack:** Rust workspace, `ratatui` 0.29, `crossterm` 0.28, `crossbeam-channel` 0.5, existing `purifier-core` crate unchanged.

---

## Design Decisions (Locked In)

- **Three-pane Miller Columns:** Parent | Current | Preview
- **Rich preview pane replaces 4-tab views:** safety verdict, type breakdown, age breakdown, size bar — all contextual to selection
- **Sort cycling within columns:** `s` cycles Size → Safety → Age → Name; filesystem hierarchy stays the same, only order changes
- **Both delete modes:** `d` for quick single-item confirm (in preview pane), `Space` to mark, `x` to execute batch
- **Settings in preview pane:** press `,` anytime → settings renders in the right pane; columns stay visible but dimmed
- **Standalone onboarding screen:** full screen before dir picker on first launch, not a modal overlay
- **Settings available anytime:** not gated on scan completion
- **LLM connection feedback:** visible timeout, specific error messages (401, timeout, etc.), classification count in status bar
- **Event loop:** input drained before draw, scan progress coalesced per frame, LLM results capped per frame (from worktree)

---

## File Structure

### Create

- `crates/purifier-tui/src/columns.rs` — Column state model: `ColumnStack` (vec of `ColumnState`), per-column path/selection/scroll, navigation methods (enter, back, jump), sort key management.
- `crates/purifier-tui/src/marks.rs` — Mark tracking for batch deletion: `MarkSet` wrapping `HashSet<PathBuf>`, toggle/clear/total-size/is-marked queries, iteration for batch confirmation view.
- `crates/purifier-tui/src/ui/columns_view.rs` — Three-pane column rendering: parent column, current column, layout proportions, per-row rendering (name, size, safety badge, mark indicator), column headers with directory name and total size.
- `crates/purifier-tui/src/ui/preview_pane.rs` — Preview pane rendering with multiple content modes: directory analytics (type/age/safety breakdowns with bar charts), file detail view, quick-delete confirmation, batch-delete review list, settings form, LLM connection status.
- `crates/purifier-tui/src/ui/onboarding.rs` — Standalone first-launch onboarding screen: centered card with provider selection (1-4), API key input, save/skip actions.

### Modify (heavy rewrite)

- `crates/purifier-tui/src/app.rs` — Replace tree-flattening model with column-stack state. Remove `View` enum, `FlatEntry`, `SizedEntry`, `rebuild_flat_entries()`, `flatten_by_*()` methods. Add `ColumnStack`, `MarkSet`, `SortKey` enum, `PreviewMode` enum (Analytics, DeleteConfirm, BatchReview, Settings, Onboarding). Keep `AppConfig`, `SettingsDraft`, `AppModal` for settings/onboarding. Keep `ScanStatus`, `LlmStatus`. Keep `DeleteStats`.
- `crates/purifier-tui/src/input.rs` — Rewrite for column navigation: `h`/`l` for column enter/back, `j`/`k` for within-column movement, `g`/`G` for top/bottom, `d` for quick delete, `Space` for mark toggle, `x` for batch execute, `s` for sort cycle, `i` for size mode toggle, `,` for settings, `/` for filter, `~` for home. Remove view-switching (1-4 keys) and tree expand/collapse logic.
- `crates/purifier-tui/src/main.rs` — Rewrite event loop carrying over worktree improvements: drain ready input before draw, cap scan event processing per frame, coalesce progress snapshots, cap LLM result application per frame. Replace tree-building scan processing with populating the `FileEntry` tree that columns index into. Adapt LLM result application to update entries in-place (badges update live in whichever column is viewing that directory).
- `crates/purifier-tui/src/ui/mod.rs` — New layout dispatcher: `MillerLayout` (three proportional columns + status bar), route to `columns_view` + `preview_pane` + `status_bar`. Remove `MainLayout` and tree-view dispatch. Keep `format_size`, `truncate_start`, `truncate_tail` helpers.

### Modify (lighter changes)

- `crates/purifier-tui/src/ui/status_bar.rs` — Adapt status bar: left shows current path breadcrumb, center shows mark count + total size (when marks exist), right shows sort mode + scan status + LLM status with classification count.
- `crates/purifier-tui/src/config.rs` — Add `SortKey` persistence to `UiConfig`. Remove `View` persistence (no longer needed). Keep provider, size mode, scan profile, onboarding config unchanged.
- `crates/purifier-tui/src/secrets.rs` — Unchanged.

### Remove

- `crates/purifier-tui/src/ui/tree_view.rs` — Replaced entirely by `columns_view.rs` + `preview_pane.rs`.
- `crates/purifier-tui/src/ui/settings_modal.rs` — Settings rendering moves into `preview_pane.rs` as a preview mode.
- `crates/purifier-tui/src/ui/dir_picker.rs` — Dir picker functionality absorbed into onboarding or kept as a minimal pre-scan screen (decision in Task 9).

### Unchanged

- All `purifier-core` files: `scanner.rs`, `classifier.rs`, `llm.rs`, `rules.rs`, `provider.rs`, `filters.rs`, `size.rs`, `types.rs`, `lib.rs`.
- `rules/default.toml`.

### Update (docs)

- `README.md` — Rewrite keybindings table, views section, and screenshots for Miller Columns.
- `PROGRESS.md` — Update architecture and current status.
- `AGENTS.md` — Update interaction model expectations.

---

## Task 1: Column State Model

**Files:**
- Create: `crates/purifier-tui/src/columns.rs`
- Modify: `crates/purifier-tui/src/app.rs` (add module, initial integration)

**Work:**
- [ ] Define `SortKey` enum: `Size`, `Safety`, `Age`, `Name` with `cycle()` method and `label()` for display.
- [ ] Define `ColumnState` struct: `path: PathBuf`, `selected_index: usize`, `scroll_offset: usize`.
- [ ] Define `ColumnStack` struct: `columns: Vec<ColumnState>`, `sort_key: SortKey`, with methods:
  - `current(&self) -> &ColumnState` — the column the user is navigating in.
  - `parent(&self) -> Option<&ColumnState>` — one level up, rendered in the left pane.
  - `enter(&mut self, path: PathBuf)` — push a new column for the selected directory.
  - `back(&mut self)` — pop current column, return to parent.
  - `move_selection(&mut self, delta: isize)` — move j/k within current column.
  - `jump_top(&mut self)`, `jump_bottom(&mut self, count: usize)` — g/G navigation.
  - `current_path(&self) -> &Path` — full path to the currently viewed directory.
  - `breadcrumb(&self) -> String` — path string for status bar display.
- [ ] Define `sorted_children()` free function: given a `&[FileEntry]` and `SortKey` and `SizeMode`, return a `Vec<usize>` of indices sorted by the key. This is the core operation — sorting one directory's children, not the whole tree.
- [ ] Register `columns` module in `app.rs` or `main.rs`.

**Tests:**
- [ ] `ColumnStack::enter` pushes a new column and `back` pops it.
- [ ] `move_selection` clamps to valid range.
- [ ] `sorted_children` sorts by each key correctly.
- [ ] `breadcrumb` returns expected path string.

**Verify:**
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-tui --all-targets -- -D warnings`

---

## Task 2: Mark Set for Batch Deletion

**Files:**
- Create: `crates/purifier-tui/src/marks.rs`

**Work:**
- [ ] Define `MarkSet` struct wrapping `HashSet<PathBuf>` with methods:
  - `toggle(&mut self, path: &Path)` — add if absent, remove if present.
  - `is_marked(&self, path: &Path) -> bool`.
  - `clear(&mut self)`.
  - `count(&self) -> usize`.
  - `total_size(&self, entries: &[FileEntry], mode: SizeMode) -> u64` — sum sizes of all marked entries by walking the entry tree.
  - `iter(&self) -> impl Iterator<Item = &PathBuf>`.
  - `remove(&mut self, path: &Path)` — for unmark-individual in batch review.
- [ ] Register module.

**Tests:**
- [ ] Toggle adds then removes.
- [ ] `total_size` sums correctly across a mock entry tree.
- [ ] `clear` empties the set.

**Verify:**
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-tui --all-targets -- -D warnings`

---

## Task 3: App State Rewrite

**Files:**
- Modify: `crates/purifier-tui/src/app.rs` (heavy rewrite)
- Modify: `crates/purifier-tui/src/config.rs` (minor)

**Work:**
- [ ] Define `PreviewMode` enum: `Analytics`, `DeleteConfirm(PathBuf)`, `BatchReview`, `Settings(SettingsDraft)`, `Onboarding(SettingsDraft)`.
- [ ] Define `AppScreen` enum: `Onboarding`, `DirPicker`, `Main`.
- [ ] Rewrite `App` struct to hold:
  - `entries: Vec<FileEntry>` — the scanned tree (same as current, unchanged from core).
  - `columns: ColumnStack` — navigation state.
  - `marks: MarkSet` — batch deletion marks.
  - `preview_mode: PreviewMode` — what the right pane shows.
  - `screen: AppScreen` — current screen.
  - `scan_status: ScanStatus`, `llm_status: LlmStatus` — carried forward.
  - `preferences: AppConfig` — carried forward.
  - `delete_stats: DeleteStats` — carried forward.
  - `scan progress fields` — `files_scanned`, `bytes_found`, `current_scan_dir`.
- [ ] Remove: `View` enum, `FlatEntry`, `SizedEntry`, `flat_entries: Vec<FlatEntry>`, `selected_index`, `expanded_paths`, all `rebuild_flat_entries()` and `flatten_by_*()` methods.
- [ ] Add `children_at_path(&self, path: &Path) -> Option<&[FileEntry]>` — returns the children of a directory given its path. This is the key accessor for column rendering.
- [ ] Add `entry_at_path(&self, path: &Path) -> Option<&FileEntry>` — look up a single entry.
- [ ] Add `selected_entry(&self) -> Option<&FileEntry>` — get the entry currently highlighted in the current column, using `columns.current()` state + `children_at_path`.
- [ ] Add `SortKey` to `UiConfig` for persistence. Remove `View` from `UiConfig`.
- [ ] Keep `SettingsDraft`, `start_scan_with_path()`, `open_onboarding()`, `open_settings()`, deletion helpers.

**Tests:**
- [ ] `children_at_path` returns correct children for root and nested paths.
- [ ] `selected_entry` returns the correct entry based on column state.
- [ ] `App::new` initializes with correct defaults.

**Verify:**
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-tui --all-targets -- -D warnings`

---

## Task 4: Column Rendering

**Files:**
- Create: `crates/purifier-tui/src/ui/columns_view.rs`
- Modify: `crates/purifier-tui/src/ui/mod.rs` (new layout)

**Work:**
- [ ] Define `MillerLayout` with three horizontal chunks: parent (flex ~0.8), current (flex ~1.1), preview (flex ~1.4). Plus status bar (1 line) at bottom and sort indicator (1 line) at top.
- [ ] Implement `render_parent_column(f, area, app)` — renders the parent directory's children. Highlight the entry that corresponds to the current column's directory. Show name + size for each entry. Dimmed styling.
- [ ] Implement `render_current_column(f, area, app)` — renders the current directory's sorted children. Each row shows: mark indicator (`✘` if marked), name, safety badge (`✓`/`⚠`/`✗`/`?`), size. Highlighted row has a distinct background. Handle viewport scrolling (only render visible rows based on `scroll_offset` and area height).
- [ ] Implement `render_sort_indicator(f, area, app)` — shows `Sort: [Size ▼]` with the active key highlighted.
- [ ] Define per-row rendering: indent-free (no tree nesting), directory entries show trailing `/`, files show extension. Safety badge colored (green/yellow/red/gray). Size right-aligned in cyan.
- [ ] Update `ui/mod.rs`: replace `MainLayout` with `MillerLayout`. Remove tree-view dispatch. Route to `columns_view` functions + `preview_pane` (Task 5) + `status_bar`.
- [ ] Keep `format_size`, `truncate_start`, `truncate_tail` helpers in `ui/mod.rs`.

**Tests:**
- [ ] Verify `sorted_children` integration with rendering (correct order based on sort key).
- [ ] Viewport scrolling: selected item always visible.

**Verify:**
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-tui --all-targets -- -D warnings`

---

## Task 5: Rich Preview Pane

**Files:**
- Create: `crates/purifier-tui/src/ui/preview_pane.rs`

**Work:**
- [ ] Implement `render_preview(f, area, app)` — dispatch on `app.preview_mode`:
  - `Analytics` → render analytics view for `app.selected_entry()`.
  - `DeleteConfirm(path)` → render quick-delete confirmation.
  - `BatchReview` → render batch review list.
  - `Settings(draft)` → render settings form.
  - `Onboarding(draft)` → render onboarding (only used in onboarding screen context).
- [ ] **Analytics view for directories:**
  - Safety verdict + colored badge + reason text.
  - Total size (logical + physical if they differ).
  - Proportional size bar (% of parent directory).
  - "By type" section: aggregate children by `Category`, show mini bar chart per category.
  - "By age" section: bucket children into age ranges (>90d, 30–90d, <30d), show mini bar chart.
  - Children count.
- [ ] **Analytics view for files:**
  - Safety verdict + reason.
  - Size (logical and physical).
  - Category.
  - Last modified date (human-readable relative + absolute).
  - File path (full, not truncated).
- [ ] **Quick-delete confirmation:**
  - Red border.
  - Full path, logical size, estimated physical freed, safety verdict + reason.
  - `[y] Delete  [n] Cancel` buttons.
- [ ] **Batch review:**
  - Title: "Delete N marked items?"
  - Scrollable list of all marked items with path, size, safety badge.
  - Total logical size, estimated physical freed.
  - `[y] Delete all  [n] Cancel  [j/k] Unmark individual`
- [ ] Helper: `aggregate_by_category(children: &[FileEntry], mode: SizeMode) -> Vec<(Category, u64)>` — sorted by size descending.
- [ ] Helper: `aggregate_by_age(children: &[FileEntry]) -> Vec<(&str, u64)>` — bucketed into age ranges.

**Tests:**
- [ ] `aggregate_by_category` groups and sorts correctly.
- [ ] `aggregate_by_age` buckets correctly.

**Verify:**
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-tui --all-targets -- -D warnings`

---

## Task 6: Input Handling Rewrite

**Files:**
- Modify: `crates/purifier-tui/src/input.rs` (heavy rewrite)

**Work:**
- [ ] Rewrite `handle_key()` dispatch based on `app.screen` and `app.preview_mode`:
  - **Main screen, Analytics preview:**
    - `h` / `Left` → `columns.back()`, reset preview to Analytics.
    - `l` / `Right` / `Enter` → if selected is dir, `columns.enter(path)`, reset preview to Analytics. If file, no-op (preview shows file detail).
    - `j` / `Down` → `columns.move_selection(1)`, refresh preview.
    - `k` / `Up` → `columns.move_selection(-1)`, refresh preview.
    - `g` → `columns.jump_top()`. `G` → `columns.jump_bottom(count)`.
    - `d` → set `preview_mode = DeleteConfirm(selected_path)`.
    - `Space` → `marks.toggle(selected_path)`.
    - `x` → if marks non-empty, set `preview_mode = BatchReview`.
    - `u` → `marks.clear()`.
    - `s` → `columns.sort_key.cycle()`.
    - `i` → toggle size mode in preferences.
    - `,` → set `preview_mode = Settings(SettingsDraft::from(preferences))`.
    - `~` → navigate columns to home directory.
    - `/` → enter filter mode for current column (stretch goal, can defer).
    - `q` / `Esc` → quit.
  - **Main screen, DeleteConfirm preview:**
    - `y` → execute deletion, remove entry, update stats, reset preview to Analytics.
    - `n` / `Esc` → reset preview to Analytics.
  - **Main screen, BatchReview preview:**
    - `y` → execute all marked deletions in order, update stats, clear marks, reset preview.
    - `n` / `Esc` → reset preview to Analytics.
    - `j`/`k` → scroll through batch list.
    - `Space` → unmark individual item from batch list.
  - **Main screen, Settings preview:**
    - `1`-`4` → select provider.
    - `a` → edit API key (text input mode).
    - `m` → toggle size mode.
    - `p` → cycle scan profile.
    - `Enter` → save settings, trigger runtime refresh, reset preview to Analytics.
    - `Esc` → cancel, reset preview to Analytics.
  - **DirPicker screen:** Keep existing dir picker input logic (or simplify).
  - **Onboarding screen:** Provider selection (1-4), API key input (a), Enter to save + proceed to DirPicker, Esc to skip.
- [ ] Remove all tree expand/collapse logic.
- [ ] Remove view-switching (1-4 number keys in main screen).

**Tests:**
- [ ] `h` on root column is a no-op (can't go above root).
- [ ] `l` on a file doesn't enter (stays on file, shows file preview).
- [ ] `d` sets DeleteConfirm, `n` resets to Analytics.
- [ ] `Space` toggles mark, `x` enters BatchReview.
- [ ] `,` opens settings, `Esc` cancels back to Analytics.

**Verify:**
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-tui --all-targets -- -D warnings`

---

## Task 7: Event Loop Rewrite

**Files:**
- Modify: `crates/purifier-tui/src/main.rs` (heavy rewrite)

**Work:**
- [ ] Carry over from `interactive-view-model` worktree:
  - Drain all ready input events before drawing.
  - Cap scan event processing per frame (e.g. 1000 events max).
  - Coalesce progress snapshots to newest per frame.
  - Cap LLM result application per frame (e.g. 50 results max).
- [ ] Adapt scan processing: scan results still build the `FileEntry` tree in `app.entries`. No flat-entry rebuilding needed — columns read from the tree directly via `children_at_path`.
- [ ] Adapt LLM result application: find the entry by path in the tree, update its `category`, `safety`, `safety_reason` in-place. The next frame render will pick up the new badge automatically since columns read from the live tree.
- [ ] Reduce idle poll to 16ms (60fps) or use event-driven wakeup if crossterm supports it. The worktree used 50ms; 16ms will feel more responsive for navigation.
- [ ] Keep scan worker thread, LLM worker thread, and channel architecture from current `main.rs`. These are well-structured already.
- [ ] Adapt startup flow: Onboarding screen → DirPicker → scan → Main (columns).
- [ ] Keep runtime provider refresh after settings save.

**Tests:**
- [ ] Scan processing populates entries tree correctly (existing tests may cover this).
- [ ] LLM result application updates the correct entry in-place.

**Verify:**
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-tui --all-targets -- -D warnings`
- Manual: run `cargo run -- ~/Downloads`, verify columns render, navigation works, no lag on expand.

---

## Task 8: Settings in Preview Pane

**Files:**
- Modify: `crates/purifier-tui/src/ui/preview_pane.rs` (add settings rendering)
- Remove: `crates/purifier-tui/src/ui/settings_modal.rs`

**Work:**
- [ ] Implement `render_settings(f, area, draft, llm_status)` inside `preview_pane.rs`:
  - "Settings" header.
  - LLM Provider row: `1:OpenRouter  2:OpenAI  3:Anthropic  4:Google` with active highlighted.
  - API Key row: masked key with `[a] edit` hint.
  - Model row: provider-derived read-only value.
  - Size Mode row: `Physical / Logical` with active highlighted and `[m] toggle` hint.
  - Scan Profile row: active profile name with `[p] cycle` hint.
  - LLM status indicator: `● Connected` / `● Connecting...` / `● Connection failed` with appropriate color.
  - Footer: `[Enter] Save  [Esc] Cancel`.
  - Storage note: `~/.config/purifier/config.toml · Keychain`.
- [ ] When settings preview is active, dim the parent and current columns (apply a dimmed style modifier in `columns_view` rendering based on `app.preview_mode`).
- [ ] Settings available regardless of `scan_status` — remove the scan-complete gate.
- [ ] Delete `settings_modal.rs`.

**Tests:**
- [ ] Settings draft initializes correctly from current preferences.
- [ ] Provider switch updates draft correctly.

**Verify:**
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-tui --all-targets -- -D warnings`

---

## Task 9: Onboarding Screen

**Files:**
- Create: `crates/purifier-tui/src/ui/onboarding.rs`
- Modify: `crates/purifier-tui/src/main.rs` (startup routing)
- Evaluate: `crates/purifier-tui/src/ui/dir_picker.rs` — keep, simplify, or merge.

**Work:**
- [ ] Implement standalone onboarding screen (full terminal, not a modal overlay):
  - Centered card with "Welcome to Purifier" header.
  - "LLM Classification (optional)" section with explanation text.
  - Provider selection: 1-4 number keys.
  - API key input: `a` to edit, shown as masked field.
  - `[Enter]` Save & proceed to dir picker.
  - `[Esc]` Skip — proceed with rules-only classification.
- [ ] Onboarding shown only when `preferences.onboarding.show_onboarding` is true (first launch).
- [ ] After onboarding save/skip, transition to DirPicker screen.
- [ ] Decision on dir picker: keep it as a minimal pre-scan screen. The existing dir picker logic (common directories, custom path input) is useful and orthogonal to the column rewrite. Move rendering from `dir_picker.rs` into the new UI dispatch in `ui/mod.rs`, or keep `dir_picker.rs` with minor styling updates to match the new visual language.

**Tests:**
- [ ] Onboarding skip sets `show_onboarding = false` and transitions to DirPicker.
- [ ] Onboarding save persists provider + key and transitions.

**Verify:**
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-tui --all-targets -- -D warnings`

---

## Task 10: Status Bar Adaptation

**Files:**
- Modify: `crates/purifier-tui/src/ui/status_bar.rs`

**Work:**
- [ ] Rewrite status bar layout to three sections:
  - **Left:** current path breadcrumb from `columns.breadcrumb()`.
  - **Center:** mark indicator — `"N marked · X MB"` in red when marks exist, empty otherwise.
  - **Right:** `Sort: Size ▼` + scan status + LLM status.
- [ ] LLM status shows classification count during active classification: `"LLM: classifying 24 paths..."`.
- [ ] LLM status shows final count when done: `"LLM ✓ · 142 paths classified"`.
- [ ] Scan status during scan: `"Scanning: 12,340 entries · 4.2 GB"`.
- [ ] Scan status after completion: `"Scan complete · 14.2 GB in 45,000 entries"`.

**Tests:**
- [ ] Breadcrumb renders correctly for deep paths.
- [ ] Mark indicator hidden when no marks, visible with correct count/size when marks exist.

**Verify:**
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-tui --all-targets -- -D warnings`

---

## Task 11: LLM Connection Flow

**Files:**
- Modify: `crates/purifier-tui/src/main.rs` (connection feedback)
- Modify: `crates/purifier-tui/src/app.rs` (status tracking)

**Work:**
- [ ] Add `LlmConnectionDetail` to track specific error types: `Timeout`, `Unauthorized`, `NetworkError(String)`, `Unknown(String)`.
- [ ] When LLM validation fails, store the specific error detail in app state.
- [ ] Preview pane renders specific error messages: `"401 Unauthorized — check your API key"`, `"Connection timed out after 10s"`, etc.
- [ ] Status bar shows connection state during validation: `"LLM: validating..."` (yellow).
- [ ] Add visible timeout: validation attempt shows countdown or timeout duration in the preview pane during validation.
- [ ] After failed validation, status bar shows: `"LLM ✗ · rules only"`. Preview pane shows detailed error + `"Press , to open settings"`.

**Tests:**
- [ ] Error detail stored correctly for different failure modes.
- [ ] LLM status transitions: Connecting → Ready or Connecting → Error.

**Verify:**
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-tui --all-targets -- -D warnings`

---

## Task 12: Cleanup and Documentation

**Files:**
- Remove: `crates/purifier-tui/src/ui/tree_view.rs`
- Remove: `crates/purifier-tui/src/ui/settings_modal.rs`
- Modify: `README.md`
- Modify: `PROGRESS.md`
- Modify: `AGENTS.md`

**Work:**
- [ ] Delete `tree_view.rs` and `settings_modal.rs`. Ensure no remaining imports.
- [ ] Update `README.md`:
  - Replace keybindings table with new Miller Columns keybindings.
  - Replace "Views" section — describe three-pane navigation + rich preview instead of 4 tabs.
  - Update architecture diagram (new TUI files).
  - Update usage examples if needed.
- [ ] Update `PROGRESS.md`:
  - Update architecture section with new column model.
  - Update "What Works Now" to reflect Miller Columns.
  - Remove references to flat entry rebuilding and 4-tab views.
  - Update known limits.
- [ ] Update `AGENTS.md`:
  - Replace interaction model expectations (no more tree expand/collapse).
  - Update verification commands if needed.
  - Note that non-size views no longer exist as separate tabs.

**Verify:**
- `cargo test -p purifier-core`
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings`
- Manual: full scan of `~/`, navigate, mark, delete, verify no panics or lag.

---

## Verification Plan

After all tasks are complete, run the full verification sequence:

```bash
# Unit tests
cargo test -p purifier-core
cargo test -p purifier-tui

# Lint
cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings

# Manual smoke tests
cargo run -- ~/Downloads      # small scan, verify columns render
cargo run -- ~/Library        # medium scan, verify sort modes work
cargo run -- /                # full scan, verify no lag on column navigation
cargo run -- --no-llm ~/tmp   # verify rules-only mode works
```

Manual checklist:
- [ ] Columns render with correct proportions.
- [ ] `h`/`l` navigation enters and exits directories smoothly.
- [ ] `j`/`k` scrolls within columns with no lag.
- [ ] Sort cycling (`s`) reorders current column correctly.
- [ ] Preview pane shows type/age/safety breakdowns for directories.
- [ ] Preview pane shows file details for files.
- [ ] Quick delete (`d` → `y`) removes entry and updates columns.
- [ ] Mark (`Space`) toggles mark indicator, persists across navigation.
- [ ] Batch delete (`x` → `y`) removes all marked entries.
- [ ] Settings (`,`) renders in preview pane, save triggers runtime refresh.
- [ ] Onboarding appears on first launch, skip works.
- [ ] LLM badges update live during classification.
- [ ] Status bar shows breadcrumb, marks, scan/LLM status.
- [ ] `q` / `Esc` quits cleanly.

---

## Migration Notes

- The `interactive-view-model` worktree contains event loop improvements that should be cherry-picked conceptually (not git-cherry-picked, since the file structure changes). Specifically: input-before-draw ordering, per-frame caps, progress coalescing.
- The dirty state on `main` (partial responsiveness patches in `app.rs` and `tree_view.rs`) can be discarded — the files are being rewritten.
- `purifier-core` is intentionally untouched. The `FileEntry` tree with `children: Vec<FileEntry>` is the data structure that columns index into. No new core types needed.
- The `expanded` field on `FileEntry` in `purifier-core/src/types.rs` becomes unused by the TUI (Miller Columns don't use expand/collapse). It can be left in place for now to avoid a core change, or removed in a follow-up.
