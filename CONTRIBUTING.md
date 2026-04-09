# Contributing to Purifier

Purifier welcomes contributions from humans, AI-assisted humans, and agent-driven workflows.

What matters here is not whether AI helped. What matters is whether the change is accurate, scoped, reviewable, and truthful about what it does.

## Good Contributions

- small, single-purpose fixes
- documentation and wording improvements
- rule additions or corrections
- UI polish that preserves current navigation and behavior
- bug reports with enough context to reproduce or narrow down the problem

## AI-Assisted Contributions Are Welcome

If you used AI to help with an issue or pull request, that is fine.

Please make sure you:

- review the output before submitting it
- remove made-up claims, stale assumptions, and vague filler
- keep screenshots, logs, and paths sanitized
- say what you actually verified, and what you did not

You do not need a perfect submission. A rough but honest report is better than a polished one that hides uncertainty.

## Project Constraints

Keep changes minimal and scoped.

Preserve the current TUI model:

- Miller Columns navigation: parent | current | preview
- no tree expand/collapse
- no flat-entry rebuild flows

Keep provider support truthful:

- live runtime-wired: `OpenRouter`, `OpenAI`
- persisted only: `Anthropic`, `Google`
- `Ollama` remains intentionally disabled

Project-specific constraints live in [AGENTS.md](AGENTS.md).

## Issues

Use the issue templates if they help. If they do not fit, open a blank issue.

Good issues usually include some of the following:

- what you were trying to do
- what you expected
- what happened instead
- whether LLM classification was enabled
- sanitized screenshots, logs, or paths when useful

Do not hold back a bug report just because you do not have every detail.

## Pull Requests

Before opening a pull request, run:

```bash
cargo test -p purifier-core
cargo test -p purifier-tui
cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings
```

Please also:

- explain the user-visible effect
- mention any privacy, deletion-safety, or provider-handling changes
- update docs when runtime behavior, privacy behavior, or user-visible flows change

Docs to keep in sync when relevant:

- [README.md](README.md)
- [PROGRESS.md](PROGRESS.md)
- [AGENTS.md](AGENTS.md)
