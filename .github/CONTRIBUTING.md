# Contributing to Purifier

Purifier is a macOS-first Rust TUI for disk cleanup with safety classification. Keep contributions scoped, truthful, and easy to verify.

## Before you change code

- Prefer small, single-purpose changes.
- Preserve the Miller Columns navigation model: parent | current | preview.
- Do not reintroduce tree expand/collapse behavior or flat-entry rebuild flows.
- Keep provider support truthful:
  - live runtime-wired: `OpenRouter`, `OpenAI`
  - persisted only: `Anthropic`, `Google`
  - `Ollama` remains intentionally disabled

Project-specific constraints are maintained in [AGENTS.md](../AGENTS.md).

## Development

Build locally:

```bash
cargo build
```

Run the required verification commands before opening a pull request:

```bash
cargo test -p purifier-core
cargo test -p purifier-tui
cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings
```

## Docs to keep in sync

When provider/runtime support, privacy behavior, or user-visible flows change, update:

- [README.md](../README.md)
- [PROGRESS.md](../PROGRESS.md)
- [AGENTS.md](../AGENTS.md)

## Pull request expectations

- Explain the user-visible effect and any behavior changes.
- List the verification commands you ran.
- Call out privacy, deletion-safety, or provider-handling changes explicitly.
- Keep screenshots and example paths sanitized before submitting.
