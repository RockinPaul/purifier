# Stabilization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task.

**Goal:** Stabilize the TUI and classification pipeline by fixing destructive bugs, wiring the missing LLM fallback, making scan behavior match the product promises, and correcting core view/rule behavior before expanding deterministic rules.

**Architecture:** Keep the current two-crate structure: `purifier-core` owns scanning, rules, and LLM integration primitives, while `purifier-tui` owns app state, event handling, and rendering. Prefer small focused helpers and app-state methods over large refactors so the current implementation becomes testable with minimal structural change.

**Tech Stack:** Rust workspace, `ratatui`, `crossterm`, `crossbeam-channel`, `reqwest`, `tokio`, `serde`, existing `purifier-core` and `purifier-tui` crates.

---

## File Scope

- `crates/purifier-tui/src/app.rs`: app state, selection identity, view flattening, error state
- `crates/purifier-tui/src/input.rs`: key handling, deletion behavior
- `crates/purifier-tui/src/main.rs`: scan event loop, LLM worker wiring, live updates
- `crates/purifier-tui/src/ui/tree_view.rs`: tree rendering, scan copy, safe truncation
- `crates/purifier-tui/src/ui/status_bar.rs`: surfaced errors and scan/LLM status
- `crates/purifier-core/src/classifier.rs`: LLM worker startup interface and batching support
- `crates/purifier-core/src/llm.rs`: OpenRouter response parsing and failure hardening
- `crates/purifier-core/src/rules.rs`: path normalization and rule matching correctness
- `README.md`: align product claims with shipped behavior

## Task 1: Fix Wrong-Entry Mutation in the TUI

**Files:**
- Modify: `crates/purifier-tui/src/app.rs`
- Modify: `crates/purifier-tui/src/input.rs`

**Work:**
- Stop using display-order-derived `entry_index` as the source of truth for mutations.
- Use a stable identity for mutations. Recommended: mutate by `PathBuf`, not by sorted index.
- Add small helpers in `App` to find, toggle, and remove entries by path.

**Tests:**
- Add `#[cfg(test)]` coverage in `app.rs` for:
  - toggling expansion on a selected entry in a sorted view mutates the intended directory
  - deleting an item after size reordering still targets the selected path

**Verify:**
- Run: `cargo test -p purifier-tui`

**Acceptance:**
- Selection identity remains stable even after size changes reorder the visible list.

## Task 2: Surface Delete Failures and Remove Unicode Truncation Panics

**Files:**
- Modify: `crates/purifier-tui/src/input.rs`
- Modify: `crates/purifier-tui/src/app.rs`
- Modify: `crates/purifier-tui/src/ui/tree_view.rs`
- Modify: `crates/purifier-tui/src/ui/status_bar.rs`

**Work:**
- Add a lightweight app error field such as `last_error: Option<String>`.
- On deletion failure, keep the item in the tree and surface the error in the UI.
- Replace all byte-slice truncation with safe truncation. Recommended minimal fix: truncate with `.chars().take(...)`.

**Tests:**
- Add tests for:
  - failed delete keeps the entry present and records an error
  - truncation helpers do not panic on non-ASCII filenames or scan paths

**Verify:**
- Run: `cargo test -p purifier-tui`

**Acceptance:**
- Delete failures are visible and Unicode paths do not crash rendering.

## Task 3: Wire the Missing LLM Fallback Path

**Files:**
- Modify: `crates/purifier-tui/src/main.rs`
- Modify: `crates/purifier-tui/src/app.rs`
- Modify: `crates/purifier-core/src/classifier.rs` only if a small API adjustment is needed

**Work:**
- Start the LLM worker when `classifier.has_llm()` is true.
- Keep rules-based classification on arrival, but batch unknown entries for LLM.
- Flush batches at 50 items and flush the remainder on scan completion.
- Consume `LlmClassification` results and apply them back to app state by path.
- Set `llm_online` from real startup state rather than leaving it permanently false.

**Implementation notes:**
- Extract small pure helpers from `main.rs` so the logic can be tested without terminal IO.
- Good helper boundaries:
  - consume one scan entry into `path_children`
  - flush pending unknowns
  - apply LLM results by path

**Tests:**
- Add tests for:
  - unknown entries are batched and flushed
  - returned LLM results update the matching path only
  - `llm_online` changes when LLM is configured

**Verify:**
- Run: `cargo test -p purifier-tui`
- Run: `cargo test -p purifier-core`

**Acceptance:**
- Unknown entries are no longer permanently stuck at `Unknown` when LLM is configured.

## Task 4: Harden `llm.rs` Against Empty or Malformed Responses

**Files:**
- Modify: `crates/purifier-core/src/llm.rs`

**Work:**
- Replace direct indexing into `choices[0]` with safe handling.
- Call `error_for_status()` before parsing.
- Treat empty `choices`, malformed JSON, and malformed payloads as recoverable failures that fall back to `Unknown` results instead of panicking.
- Extract response parsing into a small helper for unit testing.

**Tests:**
- Add tests for:
  - empty `choices`
  - JSON wrapped in markdown fences
  - malformed JSON payload
  - valid JSON payload

**Verify:**
- Run: `cargo test -p purifier-core llm`

**Acceptance:**
- LLM worker failures degrade safely instead of crashing.

## Task 5: Make Scan Results Actually Progressive

**Files:**
- Modify: `crates/purifier-tui/src/main.rs`
- Modify: `crates/purifier-tui/src/app.rs`
- Modify: `crates/purifier-tui/src/ui/tree_view.rs`

**Work:**
- Build a live snapshot during scanning instead of waiting for `ScanComplete`.
- After each frame's batch of scan events, rebuild a current tree snapshot and refresh `flat_entries`.
- Let LLM results update visible rows during the scan, not only at the end.

**Implementation note:**
- Prefer the simpler snapshot-rebuild approach first instead of a more invasive incremental tree-maintenance refactor.

**Tests:**
- Add tests for:
  - entries become visible before `ScanComplete`
  - LLM updates change visible rows while scanning
  - final `ScanComplete` state matches the progressive snapshot state

**Verify:**
- Run: `cargo test -p purifier-tui`

**Acceptance:**
- The UI shows live results while the scan is still running.

## Task 6: Make Scan Copy Truthful About `q`

**Files:**
- Modify: `crates/purifier-tui/src/ui/tree_view.rs`
- Modify: `README.md`

**Work:**
- Keep current quit behavior and update the UI copy from “Press q to cancel” to “Press q to quit”.
- Update docs if they imply cancellation semantics.

**Verify:**
- Run: `cargo test -p purifier-tui`
- Run: `cargo test`

**Acceptance:**
- On-screen copy and actual behavior match.

## Task 7: Fix the Age View

**Files:**
- Modify: `crates/purifier-tui/src/app.rs`

**Work:**
- Replace `View::ByAge => self.flatten_by_size()` with real age-based sorting.
- Recommended policy: oldest first, `None` timestamps last, size as a tiebreaker.
- Reuse the current flatten-then-sort approach to keep changes small.

**Tests:**
- Add tests for:
  - older items sort before newer items
  - missing timestamps sort last
  - size tiebreaking remains stable

**Verify:**
- Run: `cargo test -p purifier-tui`

**Acceptance:**
- The Age tab reflects modification time rather than size.

## Task 8: Fix Path Matching Correctness Before Rule Expansion

**Files:**
- Modify: `crates/purifier-core/src/rules.rs`
- Modify: `crates/purifier-tui/src/main.rs` if scan roots should be canonicalized before scanning
- Modify: `crates/purifier-core/src/scanner.rs` only if scanner-level normalization is needed

**Work:**
- Normalize scan roots and rule match inputs so relative paths classify consistently.
- Recommended approach:
  - canonicalize the chosen scan path before scanning when possible
  - in `RulesEngine::classify`, match against a normalized absolute path when possible, falling back to the raw input if normalization fails
- Do not expand the default rule set in this task.

**Tests:**
- Add tests for:
  - relative scan roots matching built-in absolute-style expectations
  - unchanged behavior for already-absolute paths
  - first-match-wins ordering still holding after normalization

**Verify:**
- Run: `cargo test -p purifier-core rules`

**Acceptance:**
- Rule classification behaves consistently for relative and absolute scan roots.

## Task 9: Final Docs and Verification Pass

**Files:**
- Modify: `README.md`
- Optionally modify: `PROGRESS.md`

**Work:**
- Reconcile README claims with actual shipped behavior after Tasks 1-8.
- Confirm the following are true and documented accurately:
  - LLM fallback is real
  - progressive UI is real
  - age view is real
  - delete errors are surfaced
  - `q` behavior wording is accurate

**Verify:**
- Run: `cargo test`
- Run: `cargo clippy --all-targets --all-features`

**Manual smoke pass:**
- Run: `cargo run -- ~/tmp` or another safe directory and verify:
  - entries appear before scan completion
  - expand/collapse works after resorting
  - delete success removes the intended item
  - delete failure shows an error
  - Unicode filenames render safely
  - unknown entries update after LLM classification when configured

**Acceptance:**
- Main branch behavior and README claims match.

---

## Execution Milestones

### Milestone A: Safety and Crash Stabilization
- Task 1: Fix wrong-entry mutation in the TUI
- Task 2: Surface delete failures and remove Unicode truncation panics

**Outcome:**
- destructive actions target the intended item
- render-time crashes from Unicode path truncation are removed
- deletion failures are user-visible

### Milestone B: LLM Feature Restoration
- Task 3: Wire the missing LLM fallback path
- Task 4: Harden `llm.rs` against empty or malformed responses

**Outcome:**
- README-promised LLM behavior actually exists
- provider-side or malformed responses degrade safely

### Milestone C: Scan and View Correctness
- Task 5: Make scan results actually progressive
- Task 6: Make scan copy truthful about `q`
- Task 7: Fix the Age view

**Outcome:**
- scan UX matches the product claims
- tab behavior is correct
- copy and runtime behavior are aligned

### Milestone D: Rule Engine Correctness and Final Verification
- Task 8: Fix path matching correctness before rule expansion
- Task 9: Final docs and verification pass

**Outcome:**
- rule matching semantics are consistent
- docs reflect reality
- the workspace has a clean verified stabilization baseline before deterministic rule expansion begins

## Post-Stabilization Follow-up

After Milestone D, begin a separate plan for deterministic rule expansion. Do not start expanding `rules/default.toml` until the stabilization tasks above are complete and verified.
