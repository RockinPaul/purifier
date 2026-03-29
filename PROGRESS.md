# Purifier — Project Progress

## What Is Purifier?

Purifier is a terminal-based disk space analyzer and cleanup tool for macOS, inspired by [GrandPerspective](https://grandperspectiv.sourceforge.io/). It scans your filesystem, shows what's using space, and — uniquely — tells you **whether each path is safe to delete and why**.

Unlike tools like `ncdu` or `dust` that only show sizes, Purifier classifies every path with a safety level (**Safe**, **Caution**, **Unsafe**, **Unknown**) and provides a human-readable explanation. For paths it doesn't recognize, it can optionally query an LLM via OpenRouter for intelligent classification.

### Target Audience

Both **developers** (cleaning build artifacts like `node_modules/`, `target/`, caches) and **general users** (finding large files, old downloads, forgotten data).

### Inspiration & Differentiation

| Existing Tool | What Purifier Adds |
|---------------|-------------------|
| GrandPerspective (macOS GUI) | Terminal-based, safety classification, explanations |
| ncdu (C, TUI) | Safety classification, LLM fallback, per-item explanations |
| dua-cli (Rust, TUI) | Safety badges, categorization, LLM for unknown paths |
| mcdu (Rust, TUI) | General-user mode (not just developer artifacts), LLM |
| dust (Rust, CLI) | Interactive TUI, deletion, safety classification |

---

## Technology Stack

| Layer | Technology | Why |
|-------|-----------|-----|
| **Language** | Rust | Performance, safety, single-binary distribution |
| **TUI framework** | [Ratatui](https://ratatui.rs/) v0.29 | Most mature Rust TUI library, rich widget set, immediate-mode rendering |
| **Terminal backend** | [Crossterm](https://github.com/crossterm-rs/crossterm) v0.28 | Cross-platform terminal I/O, works on macOS/Linux/Windows |
| **Filesystem scanner** | [jwalk](https://github.com/Byron/jwalk-rs) v0.8 | Parallel directory walking, maxes out SSD throughput |
| **Thread communication** | [crossbeam-channel](https://github.com/crossbeam-rs/crossbeam) v0.5 | Lock-free channels for scanner→TUI message passing |
| **LLM integration** | [reqwest](https://github.com/seanmonstar/reqwest) v0.12 + [OpenRouter](https://openrouter.ai/) | HTTP client for cloud LLM classification of unknown paths |
| **CLI parsing** | [clap](https://github.com/clap-rs/clap) v4 (derive) | Declarative argument parsing with help generation |
| **Rules engine** | [glob](https://github.com/rust-lang/glob) v0.3 + [toml](https://github.com/toml-rs/toml) v0.8 | Pattern matching for safety rules, TOML config format |
| **Path resolution** | [dirs](https://github.com/dirs-dev/dirs-rs) v6 | macOS-aware home directory and standard path resolution |
| **Serialization** | [serde](https://serde.rs/) v1 + serde_json | Type serialization for rules, LLM API request/response |
| **Date handling** | [chrono](https://github.com/chronotope/chrono) v0.4 | File age calculations for the "By Age" view |

---

## Architecture

### Two-Crate Workspace

```
purifier/
├── Cargo.toml                    # workspace root
├── crates/
│   ├── purifier-core/            # library crate (no terminal dependency)
│   │   └── src/
│   │       ├── lib.rs            # public API + delete_entry()
│   │       ├── types.rs          # FileEntry, SafetyLevel, Category, ScanEvent
│   │       ├── scanner.rs        # parallel filesystem walker
│   │       ├── rules.rs          # TOML-based safety rules engine
│   │       ├── classifier.rs     # rules + LLM orchestration
│   │       └── llm.rs            # OpenRouter API client
│   └── purifier-tui/             # binary crate (TUI frontend)
│       └── src/
│           ├── main.rs           # CLI parsing, terminal setup, event loop
│           ├── app.rs            # application state, dir picker, view logic
│           ├── input.rs          # keybinding handler (picker + main + delete)
│           └── ui/
│               ├── mod.rs        # screen dispatch, tab bar, delete popup, format_size
│               ├── dir_picker.rs # startup directory selection screen
│               ├── tree_view.rs  # directory tree + scanning progress panel
│               └── status_bar.rs # live scan progress, freed space, LLM status
├── rules/
│   └── default.toml              # ~30 macOS-specific safety rules
└── docs/
    └── superpowers/specs/
        └── 2026-03-28-purifier-tui-design.md   # full design spec
```

**Why two crates?**
- `purifier-core` has zero terminal dependencies — it's testable in isolation with unit tests
- `purifier-tui` is a thin rendering shell that consumes core's API via channels
- This separation allows future frontends (GUI, web, CI tool) without touching core logic

### Data Flow

```
                     crossbeam-channel
  jwalk (threads) ──────────────────────> main event loop
       │                                       │
       │ ScanEvent::Entry                      │ classify each entry
       │ ScanEvent::Progress                   │ via RulesEngine
       │ ScanEvent::ScanComplete               │
       │                                       ▼
       │                              ┌──────────────────┐
       │                              │ App state        │
       │                              │  - entries tree  │
       │                              │  - flat_entries  │
       │                              │  - scan progress │
       │                              └──────┬───────────┘
       │                                     │
       │                              Ratatui render
       │                                     │
       │                                     ▼
       │                              Terminal output
       │
  Unknown paths ──> batch (50) ──> OpenRouter LLM ──> update entries
```

### Key Design Decisions

1. **Immediate-mode rendering** — the entire UI is redrawn every frame from `App` state. No retained widget tree. This is Ratatui's model and keeps the code simple.

2. **Capped event drain** — the scanner produces events faster than the UI can render. The event loop processes at most 1000 scan events per frame, then always polls for keyboard input. This prevents input starvation (the original bug that made `q` unresponsive during scans).

3. **Progressive scanning** — the scanner streams `Entry` events as it walks. Progress events arrive every 500 files. The tree is built from a flat `HashMap<PathBuf, Vec<FileEntry>>` and assembled into a tree structure only on `ScanComplete`.

4. **Safety classification pipeline** — every entry passes through the rules engine first (instant, O(n) rules). Only entries with `safety: Unknown` are queued for LLM classification in batches of 50.

---

## What Has Been Built

### Core Library (`purifier-core`) — 6 modules, ~700 lines

| Module | Status | Description |
|--------|--------|-------------|
| `types.rs` | Done | `FileEntry`, `SafetyLevel` (Safe/Caution/Unsafe/Unknown), `Category` (BuildArtifact/Cache/Download/AppData/Media/System/Unknown), `ScanEvent` (Entry/Progress/ScanComplete) |
| `scanner.rs` | Done | Parallel filesystem walker using `jwalk`. Streams entries via crossbeam channel. Sends progress every 500 entries. Gracefully skips permission-denied paths. |
| `rules.rs` | Done | Loads TOML rule files, expands `~` paths, matches globs top-to-bottom. First match wins. Supports multiple rule files with priority ordering. |
| `classifier.rs` | Done | Orchestrates rules engine + optional LLM client. Classifies entries synchronously via rules, queues unknowns for async LLM batching. |
| `llm.rs` | Done | OpenRouter API client. Sends structured prompts with path, size, type, age. Parses JSON responses. Falls back to "Unknown" on any failure. Default model: `google/gemini-2.0-flash-001` (free tier). |
| `lib.rs` | Done | Public API surface + `delete_entry()` function for safe file/directory removal with size tracking. |

### TUI Frontend (`purifier-tui`) — 7 modules, ~1300 lines

| Module | Status | Description |
|--------|--------|-------------|
| `main.rs` | Done | CLI parsing (clap), terminal setup/teardown, main event loop with capped drain (1000/frame), scanner lifecycle management |
| `app.rs` | Done | `App` state struct, `AppScreen` (DirPicker/Main), `View` enum (BySize/ByType/BySafety/ByAge), tree flattening for display, directory picker option builder |
| `input.rs` | Done | Three input modes: dir picker (j/k/Enter/q + custom path via `/`), main view (vim keys, tab switching, expand/collapse), delete confirmation (y/n) |
| `ui/mod.rs` | Done | Screen dispatcher (picker vs main), tab bar rendering, delete confirmation popup with safety coloring, `format_size()` utility |
| `ui/dir_picker.rs` | Done | Full-screen centered directory picker with bordered frame, selection highlighting, custom path input field, help bar |
| `ui/tree_view.rs` | Done | Directory tree with indentation, expand/collapse arrows, file names, human-readable sizes, proportional size bars, colored safety badges. Scanning progress panel when tree is empty. |
| `ui/status_bar.rs` | Done | Live progress during scan (file count + bytes + current dir), completion summary, skip count, freed space counter, LLM connection status |

### Safety Rules (`rules/default.toml`) — ~30 rules

Covers macOS-specific paths across 5 categories:
- **Build artifacts** (13 rules): node_modules, target/debug, target/release, .build, __pycache__, .tox, .gradle, DerivedData, Pods, .dart_tool, vendor/bundle, build/outputs
- **Caches** (6 rules): ~/Library/Caches, ~/.cache, ~/.npm/_cacache, ~/.cargo/registry/cache, ~/Library/Logs, ~/.Trash
- **Downloads** (5 rules): ~/Downloads/*.dmg, *.pkg, *.zip, *.tar.gz, catch-all
- **App data** (3 rules): ~/Library/Application Support, ~/Library/Preferences, ~/Library/Containers
- **System** (5 rules): .git/objects, .git, /System, /usr, ~/Library/Keychains, ~/Documents, ~/Desktop

### Tests — 7 unit tests passing

| Test | Module | What it verifies |
|------|--------|-----------------|
| `test_scan_tempdir` | scanner | Scans a temp directory, verifies correct entries/sizes arrive via channel, handles Progress events |
| `test_rule_matching` | rules | node_modules, target/debug, .git all match expected category + safety |
| `test_unknown_path` | rules | Unrecognized path returns `None` (no match) |
| `test_multiple_rule_files` | rules | Custom rules file takes priority over defaults |
| `test_classify_known_entry` | classifier | Known path gets classified by rules engine |
| `test_classify_unknown_entry` | classifier | Unrecognized path stays Unknown when no LLM |
| `test_batch_unknowns` | classifier | 120 entries correctly batched into groups of 50 (50+50+20) |

---

## Commit History

| Commit | Description |
|--------|-------------|
| `beae4fa` | Initial commit (README only) |
| `432ecec` | Install Claude Code skills |
| `1033961` | Trim skills to Claude Code only |
| `e82251b` | Replace skill symlinks with copies |
| `8751149` | Add .gitignore |
| `f666ed4` | **Implement full Purifier TUI** — workspace, scanner, rules, classifier, LLM client, 4 tabbed views, deletion flow, status bar. 19 files, ~4500 lines. |
| `31c4e86` | Update README with full documentation |
| `ce7fd41` | **Fix scanning UX** — cap event drain to 1000/frame (fixes input starvation), add ScanProgress events every 500 entries, add startup directory picker |
| `9652eb2` | **Improve UI** — full-screen bordered dir picker, centered scanning progress panel with live file count/bytes/path |

---

## What Remains (Future Work)

### High Priority
- **End-to-end testing on macOS** — verify scanning, classification, deletion, all views on real filesystem
- **LLM integration testing** — test with actual OpenRouter API key, verify batch classification works
- **Handle large trees in UI** — scrolling/viewport for the tree view when thousands of entries exist

### Medium Priority
- **Suggested cleanup view (Tab 3)** — currently groups by safety level; should also allow batch approve/reject of entire groups
- **Search** (`/` key in main view) — filter entries by name pattern
- **Symlink handling** — detect and skip/flag symlinks to avoid double-counting
- **Config file** — persistent settings (~/.config/purifier/config.toml) for API key, default rules, preferred view

### Low Priority / Future
- **Linux support** — add Linux-specific rules (XDG paths, snap/flatpak caches, systemd journals)
- **Windows support** — Windows path conventions, AppData, temp directories
- **Local LLM fallback** — Ollama integration for offline classification
- **Export** — save scan results to JSON/CSV for analysis
- **Undo** — move to trash instead of permanent delete, with undo support

---

## How to Build & Run

```bash
# Clone
git clone https://github.com/RockinPaul/purifier.git
cd purifier

# Build
cargo build --release

# Run (shows directory picker)
./target/release/purifier

# Run with specific path (skips picker)
./target/release/purifier ~/Downloads

# Run without LLM
./target/release/purifier --no-llm

# Run with LLM classification
export OPENROUTER_API_KEY=your_key_here
./target/release/purifier

# Run tests
cargo test
```

---

## Project Stats

| Metric | Value |
|--------|-------|
| Rust source files | 13 |
| Lines of Rust | ~2,000 |
| Crates in workspace | 2 (purifier-core + purifier-tui) |
| Dependencies | 17 direct (103 total locked) |
| Unit tests | 7 passing |
| Safety rules | ~30 (macOS-specific) |
| Commits | 9 |
| Platform | macOS (day one) |
