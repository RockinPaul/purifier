# Purifier - Project Progress

## Current Snapshot

Purifier is a macOS terminal disk cleanup tool built in Rust. It scans the filesystem, navigates directories via Miller Columns, and explains whether a path is safe to delete.

Current state:

- Core scanning, rules, deletion flow, scan profiles, and Miller Columns TUI are implemented.
- Unknown paths can be sent to an optional LLM classifier.
- Live runtime-wired providers are `OpenRouter` and `OpenAI`.
- `Anthropic` and `Google` settings can be persisted, but runtime support is not wired yet.
- `Ollama` is intentionally disabled for now. The code keeps TODO markers for restoring it later.
- User preferences are persisted in the OS config directory, for example `~/Library/Application Support/purifier/config.toml` on macOS.
- If saved from settings, provider API keys are stored in neighboring plaintext `secrets.toml` with restrictive Unix file permissions.
- Unknown-path LLM requests can send exact path strings plus kind, size, and age metadata to the configured remote provider.
- The TUI has first-launch onboarding and settings in the preview pane.
- Size mode is persisted and defaults to `Physical`.
- Built-in scan profiles are available by default: `Full scan`, `Fast developer scan`.
- Blocking scans use a responsive progress overlay; columns populate after scan completes.
- Physical totals are hard-link-aware and use allocated-block accounting on Unix/macOS.
- This branch passes:
  - `cargo test -p purifier-core`
  - `cargo test -p purifier-tui`
  - `cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings`

## Architecture

Purifier is a two-crate workspace:

- `crates/purifier-core`
  - filesystem scanner
  - size model (`logical` vs `physical`)
  - scan profiles and filters
  - TOML safety rules engine
  - provider-neutral classification types
  - runtime LLM clients and batching helpers
- `crates/purifier-tui`
  - CLI parsing and startup resolution
  - **Miller Columns** navigation model (ColumnStack, SortKey)
  - Mark set for batch deletion
  - Preview pane with analytics, delete confirm, batch review, settings
  - Persisted app/config state (sort key, size mode, scan profile)
  - Standalone onboarding screen
  - Status bar with breadcrumb, marks, sort/scan/LLM status
  - Ratatui rendering

Important current files:

- `crates/purifier-core/src/filters.rs`
- `crates/purifier-core/src/size.rs`
- `crates/purifier-core/src/provider.rs`
- `crates/purifier-core/src/llm.rs`
- `crates/purifier-core/src/classifier.rs`
- `crates/purifier-core/src/scanner.rs`
- `crates/purifier-tui/src/main.rs`
- `crates/purifier-tui/src/app.rs`
- `crates/purifier-tui/src/columns.rs`
- `crates/purifier-tui/src/marks.rs`
- `crates/purifier-tui/src/input.rs`
- `crates/purifier-tui/src/config.rs`
- `crates/purifier-tui/src/secrets.rs`
- `crates/purifier-tui/src/ui/columns_view.rs`
- `crates/purifier-tui/src/ui/preview_pane.rs`
- `crates/purifier-tui/src/ui/status_bar.rs`
- `crates/purifier-tui/src/ui/onboarding.rs`

## Provider Status

| Provider | Status | Notes |
|----------|--------|-------|
| `OpenRouter` | Live | Default remote LLM path |
| `OpenAI` | Live | Runtime-wired and available from settings/CLI |
| `Anthropic` | Saved only | Config can be persisted; runtime client not wired yet |
| `Google` | Saved only | Config can be persisted; runtime client not wired yet |
| `Ollama` | Disabled | Intentionally removed from runtime/CLI for now; restore later via TODOs |

## What Works Now

- Parallel filesystem scanning via `jwalk`
- Physical size accounting using allocated blocks on Unix/macOS
- Logical/physical size toggle in persisted UI state
- Hard-link-aware physical totals
- Rule-based classification for common macOS cleanup paths
- LLM fallback batching for unknown entries
- Responsive blocking scan progress with overlay until completion
- Background scan processing with coalesced progress snapshots
- Scan profiles with scan-time exclusion
- Built-in profiles: `Full scan`, `Fast developer scan`
- **Miller Columns navigation** (parent | current | preview)
- **Sort cycling** within columns (Size → Safety → Age → Name)
- **Rich preview pane** with type/age/safety breakdowns for directories, file details for files
- **Quick delete** (d → confirm in preview) and **batch mark-and-sweep** (Space → x → confirm)
- Settings rendered in preview pane, available anytime (not gated on scan completion)
- Standalone onboarding screen for first launch
- Status bar with breadcrumb, mark count, sort mode, scan/LLM status
- Safe delete confirmation flow with logical remove size plus estimated/observed physical freed-space reporting
- Persistent sort key and last scan path
- In-app warning vs error messaging

## Accepted Current Behavior

- If an LLM returns partial or path-misaligned results, some rows may remain at `Analyzing with LLM...`.
- This is currently acceptable because it is less misleading than claiming a successful or failed classification for the wrong path.

## Known Limits

- `Anthropic` and `Google` are not runtime-wired yet.
- `Ollama` is disabled for now.
- APFS clone/shared-block accounting is still best-effort; physical totals do not compute exact unique clone ownership.
- Excluded directories are currently omitted from results and totals, but scanner traversal is not yet pruned at the walker level.
- The settings form currently treats `model` and `base_url` as provider-derived, read-only values.
- Provider runtime refresh is intentionally conservative and follows launch-time CLI/env precedence.

## Next Work

- Wire runtime clients for `Anthropic` and `Google`.
- Restore `Ollama` only when the runtime/client UX can be reintroduced safely.
- Prune excluded subtrees earlier in the scanner for better large-tree scan performance.
- Add richer end-user controls for custom profile authoring and advanced filter modes.
- Improve provider-specific structured output and response reconciliation.
- Keep `README.md`, `PROGRESS.md`, and `AGENTS.md` aligned whenever provider/runtime support changes.
