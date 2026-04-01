# Purifier - Project Progress

## Current Snapshot

Purifier is a macOS terminal disk cleanup tool built in Rust. It scans the filesystem, groups entries by size/type/safety/age, and explains whether a path is safe to delete.

Current state in this branch/worktree:

- Core scanning, rules, deletion flow, scan profiles, and TUI views are implemented.
- Unknown paths can be sent to an optional LLM classifier.
- Live runtime-wired providers are `OpenRouter` and `OpenAI`.
- `Anthropic` and `Google` settings can be persisted, but runtime support is not wired yet.
- `Ollama` is intentionally disabled for now. The code keeps TODO markers for restoring it later.
- User preferences are persisted in `~/.config/purifier/config.toml`.
- Provider secrets are stored in macOS Keychain.
- The TUI has first-launch onboarding and a settings modal.
- Size mode is persisted and defaults to `Physical`.
- Built-in scan profiles are available by default: `Full scan`, `Fast developer scan`.
- Blocking scans use a responsive hidden-tree progress flow and can overlap hidden LLM batching.
- Physical totals are hard-link-aware and use allocated-block accounting on Unix/macOS.
- This worktree currently passes:
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
  - persisted app/config state
  - size-mode and scan-profile selection
  - modal/input handling
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
- `crates/purifier-tui/src/input.rs`
- `crates/purifier-tui/src/config.rs`
- `crates/purifier-tui/src/secrets.rs`
- `crates/purifier-tui/src/ui/settings_modal.rs`

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
- Responsive blocking scan progress with hidden tree until completion
- Background scan processing with coalesced progress snapshots
- Scan profiles with scan-time exclusion
- Built-in profiles: `Full scan`, `Fast developer scan`
- Size/Type/Safety/Age views
- Safe delete confirmation flow with logical remove size plus estimated/observed physical freed-space reporting
- Persistent default view and last scan path
- First-launch onboarding modal
- Settings modal with:
  - provider switching for currently exposed providers
  - API-key editing
  - size-mode toggle
  - scan-profile selection
  - safe persistence to config + Keychain
  - live runtime refresh after successful save
- In-app warning vs error messaging

## Accepted Current Behavior

- If an LLM returns partial or path-misaligned results, some rows may remain at `Analyzing with LLM...`.
- This is currently acceptable because it is less misleading than claiming a successful or failed classification for the wrong path.

## Known Limits

- `Anthropic` and `Google` are not runtime-wired yet.
- `Ollama` is disabled for now.
- APFS clone/shared-block accounting is still best-effort; physical totals do not compute exact unique clone ownership.
- Excluded directories are currently omitted from results and totals, but scanner traversal is not yet pruned at the walker level.
- The settings modal currently treats `model` and `base_url` as provider-derived, read-only values.
- Provider runtime refresh is intentionally conservative and follows launch-time CLI/env precedence.

## Next Work

- Wire runtime clients for `Anthropic` and `Google`.
- Restore `Ollama` only when the runtime/client UX can be reintroduced safely.
- Prune excluded subtrees earlier in the scanner for better large-tree scan performance.
- Add richer end-user controls for custom profile authoring and advanced filter modes.
- Improve provider-specific structured output and response reconciliation.
- Keep `README.md`, `PROGRESS.md`, and `AGENTS.md` aligned whenever provider/runtime support changes.
