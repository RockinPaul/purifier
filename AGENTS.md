# AGENTS.md

## Repository Guidance

- Keep changes minimal and scoped.
- Prefer `apply_patch` for manual edits.
- Do not revert unrelated work in the tree.

## Verification

Run these before claiming work is complete:

- `cargo test -p purifier-core`
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings`

## Provider Truth

- Live runtime-wired providers: `OpenRouter`, `OpenAI`
- Persisted but not runtime-wired yet: `Anthropic`, `Google`
- `Ollama` is temporarily disabled

Do not re-enable `Ollama` casually. Keep or add explicit TODO comments instead until the runtime/client flow is intentionally restored.

## Navigation Model

- The TUI uses **Miller Columns** (three-pane: parent | current | preview).
- There is no tree expand/collapse. Navigation enters directories (h/l) and sorts within columns (s).
- The preview pane replaces the old 4-tab view system (Size/Type/Safety/Age).
- The preview pane also hosts settings, delete confirmation, and batch review.
- Do not introduce tree-flattening, `FlatEntry`, `rebuild_flat_entries`, or similar patterns. Each column reads directly from the `FileEntry` tree via `children_at_path`.

## Settings And Onboarding

- Settings are rendered in the preview pane and available anytime (not gated on scan completion).
- Press `,` to open settings.
- Onboarding is a standalone full-screen before the dir picker on first launch.
- After successful settings save, the app refreshes runtime state in-process.
- Status/warning messaging should stay truthful about launch-time CLI/env overrides.

## Deletion

- `d` opens quick-delete confirmation in the preview pane.
- `Space` marks items for batch deletion. `x` opens batch review. `u` clears marks.
- Marks persist across column navigation.
- Both modes are available simultaneously.

## Size And Scan Truth

- Default size mode is `Physical`; `Logical` remains a user-selectable alternative.
- Physical totals are hard-link-aware and based on allocated blocks on Unix/macOS.
- APFS clone/shared-block accounting is still best-effort; do not claim exact unique physical ownership.
- The current scan UX is blocking with a progress overlay; columns populate after scan completes.
- Built-in scan profile names are reserved and should remain user-visible defaults that can evolve across releases.

## LLM Result Handling

- Current accepted behavior: if a provider returns partial or path-misaligned batch results, rows may remain at `Analyzing with LLM...`.
- Do not replace that with something more misleading unless you can reconcile results safely.

## Docs To Keep In Sync

When provider/runtime support changes, update:

- `README.md`
- `PROGRESS.md`
- `AGENTS.md`
