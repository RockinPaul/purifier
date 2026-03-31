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

## Settings And Onboarding

- Settings are only available after the scan is complete.
- Onboarding can appear before or during the initial scan.
- After successful settings save, the app refreshes runtime state in-process.
- Status/warning messaging should stay truthful about launch-time CLI/env overrides.

## LLM Result Handling

- Current accepted behavior: if a provider returns partial or path-misaligned batch results, rows may remain at `Analyzing with LLM...`.
- Do not replace that with something more misleading unless you can reconcile results safely.

## Docs To Keep In Sync

When provider/runtime support changes, update:

- `README.md`
- `PROGRESS.md`
- `AGENTS.md`
