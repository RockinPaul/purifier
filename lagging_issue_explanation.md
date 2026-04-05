# Lagging Issue Explanation

## Executive Summary

The app is still laggy because its interaction model is only partially optimized.

There are currently **three relevant code states**:

1. **Merged `main` at `5511b21`**
   This has the GrandPerspective-style size/accounting work, but not the broader responsiveness redesign.
2. **Dirty root `main` workspace**
   The root workspace currently has additional uncommitted local changes in:
   - `crates/purifier-tui/src/app.rs`
   - `crates/purifier-tui/src/ui/tree_view.rs`
   These are partial local responsiveness optimizations, not a finished redesign.
3. **Experimental worktree `.worktrees/interactive-view-model`**
   This contains a larger responsiveness redesign, also uncommitted:
   - `crates/purifier-tui/src/app.rs`
   - `crates/purifier-tui/src/input.rs`
   - `crates/purifier-tui/src/main.rs`
   - `crates/purifier-tui/src/ui/tree_view.rs`

The experimental worktree is much better structured than current `main`, and it passes verification, but it still does not guarantee that opening a **very large** directory will feel fast. It removes several global bottlenecks, but it still has proportional work in some important paths.

## Current Repository State

### `main`

- Branch: `main`
- HEAD: `5511b21 Add GrandPerspective-style size accounting and scan profiles`
- Root workspace is **not clean**.

### Experimental responsiveness worktree

- Worktree: `.worktrees/interactive-view-model`
- Branch: `interactive-view-model`
- Based on `5511b21`
- Not committed yet
- Current verification there passes:
  - `cargo test -p purifier-core`
  - `cargo test -p purifier-tui`
  - `cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings`

## Findings

### 1. Current `main` still rebuilds far too much on interaction

The current root workspace still uses a full rebuild model for directory toggles.

Relevant paths in the current root workspace:

- `crates/purifier-tui/src/app.rs:416-429`
  `toggle_expand()` mutates one node, then immediately calls `rebuild_flat_entries()`.
- `crates/purifier-tui/src/app.rs:483-584`
  `rebuild_flat_entries()` still rebuilds the entire current view by recursively flattening the whole tree.
- `crates/purifier-tui/src/app.rs:493-525`
  `flatten_by_size()` sorts and flattens the whole tree, not just the affected branch.
- `crates/purifier-tui/src/app.rs:528-584`
  `flatten_by_group()` and `flatten_by_age()` also rebuild full view state.

In other words, on current `main`, opening one directory is still fundamentally tied to rebuilding the whole visible data model.

### 2. Current `main` also has extra interaction latency in the event loop

Relevant path:

- `crates/purifier-tui/src/main.rs:271-344`

The loop still draws first, then processes scan work, then handles input via `poll(Duration::from_millis(50))`.

That means:

- user input is not prioritized
- there is still a fixed latency component from the 50 ms poll
- scan / classification traffic can delay keyboard handling

### 3. The experimental worktree removes several major bottlenecks

The worktree redesign improves four important things:

#### 3a. `BySize` no longer fully rebuilds on expand/collapse

Relevant path:

- `crates/purifier-tui/src/app.rs:535-585`

In the worktree:

- `BySize` toggles splice visible subtree rows into/out of `flat_entries`
- it does **not** call a full `rebuild_flat_entries()` for the toggle path

That is a real architectural improvement.

#### 3b. Non-size views no longer pretend to be hierarchical filesystem trees

Relevant paths:

- `crates/purifier-tui/src/app.rs:841-929`
- `crates/purifier-tui/src/input.rs`
- `crates/purifier-tui/src/ui/tree_view.rs`

In the worktree:

- `ByType`, `BySafety`, and `ByAge` are grouped flat views with collapsible section headers only
- they no longer use per-directory expansion behavior
- this removes a lot of wasted rebuild work outside `BySize`

#### 3c. Input is processed before draw and per-frame work is capped

Relevant path:

- `crates/purifier-tui/src/main.rs:275-359`

In the worktree:

- ready input is drained before draw
- scan progress is capped per frame
- LLM result application is capped per frame
- scan progress is coalesced to the newest snapshot per frame

This is much better than current `main`.

#### 3d. Rendering only builds visible rows

Relevant path:

- `crates/purifier-tui/src/ui/tree_view.rs:43-136`

The worktree only builds widgets for the visible window instead of every row in the list.

## Why the app can still feel laggy even after the redesign

This is the key point.

The experimental worktree is **not** the same as a full retained tree/view model.

### Remaining structural costs in the experimental worktree

#### 1. Expanding a huge directory is still proportional to the size of that subtree

Relevant path:

- `crates/purifier-tui/src/app.rs:807-821`

`visible_size_subtree_rows()` still builds every row that becomes visible for the selected subtree.

That is much better than rebuilding the whole tree, but if a single directory has a very large descendant set, opening it is still expensive.

This is not a bug; it is the remaining cost of materializing many rows in one frame.

#### 2. Recursive path lookup still exists on toggle

Relevant paths:

- `crates/purifier-tui/src/app.rs:621-657`

`get_entry_mut_by_path()` and `get_entry_by_path()` still recursively walk the tree by path.

That means the app still does not have the fully retained `NodeId` / indexed-node architecture used by the fastest Rust file browsers.

#### 3. Any call to `rebuild_flat_entries()` still rebuilds retained size totals for the whole tree

Relevant path:

- `crates/purifier-tui/src/app.rs:660-713`

The redesign introduced a retained `size_totals` cache, but `rebuild_flat_entries()` still repopulates that cache from the entire tree.

So grouped-view regrouping and some async update paths still have a full-tree traversal cost.

#### 4. There is still a 50 ms poll in the idle loop

Relevant path:

- `crates/purifier-tui/src/main.rs:377-380`

Even though ready input is drained before draw, the loop still falls back to `poll(Duration::from_millis(50))` after drawing.

That means a brand-new key event arriving just after draw can still pick up noticeable latency.

This is smaller than the old problem, but it is still part of the feel of the app.

## Current code-review conclusion

### On `main`

The root cause is straightforward:

- one directory toggle still triggers a full view rebuild
- non-size views still pay more rebuild cost than they should
- input handling is still later in the frame than it should be

### On the experimental worktree

The redesign is directionally correct and testable, but it is still only a **partial** architecture shift.

It has removed:

- full-tree rebuilds from the `BySize` toggle path
- fake directory expansion in non-size views
- unbounded ready-input drain
- stale scan-progress overwrites
- unbounded LLM result bursts

It has **not** yet removed:

- recursive path lookup
- subtree-materialization cost on very large opens
- full-tree size-cache rebuilds in regrouping paths
- the remaining 50 ms poll-driven latency component

## Research Notes: Similar Rust Projects

The Rust projects surveyed earlier point toward the same durable pattern:

- `yazi`
  - retained per-folder state
  - cached visible projections
  - async/background updates applied as patches
- `broot`
  - stable line store over retained nodes
  - splice lines instead of rebuilding from root
- `joshuto`
  - cached directory state plus viewport state
- `dua-cli` / similar disk tools
  - retained graph/tree metadata and local recomputation

Purifier’s experimental worktree has borrowed some of that, but it still does not have the full retained indexed tree model.

## What the project currently is

As of now, the project has:

- merged GrandPerspective-style physical/logical size support
- hard-link-aware physical accounting
- scan profiles and built-in profiles
- truthful delete-space reporting
- provider settings/onboarding/runtime refresh work

Responsiveness work is **not merged**.

There is currently a split state:

- `main` contains the product features
- `.worktrees/interactive-view-model` contains the larger responsiveness redesign
- the root `main` workspace also has a smaller, partial uncommitted responsiveness patch in `app.rs` and `tree_view.rs`

That means the repo is currently in an awkward but understandable place:

- the product surface evolved faster than the interaction architecture
- some performance work exists in local-only branches/worktrees
- the fully durable performance model has not been finished or merged

## Bottom Line

The app still lags because the architecture is only half-transitioned.

The current merged `main` is still fundamentally rebuild-oriented.

The experimental redesign fixes a lot, but it is still not the full retained-node model needed to make large-directory open/close interactions feel consistently fast.

### The most honest summary

- If you are testing current `main`, the lag is expected from the code.
- If you are testing the experimental worktree, the lag is reduced structurally, but very large subtree opens can still be expensive because the app still has to build every newly visible row in that subtree and still lacks a `NodeId`-indexed retained tree model.

## Recommended next step

If the goal is truly "it must respond much faster," the next real milestone is:

1. introduce a retained indexed tree model (`NodeId`, parent/child ids, path index)
2. stop recursive path lookup by `PathBuf`
3. stop rebuilding size totals from the whole tree during regrouping paths
4. make all non-size view updates patch cached row stores incrementally
5. remove or substantially reduce the remaining 50 ms idle poll latency

Until that is done, improvements will continue to be meaningful but incomplete.
