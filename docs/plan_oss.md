# Open Source Readiness Assessment

## Status On 2026-04-09

This document now reflects the current working tree state rather than the original first-pass audit.

## Completed In The Current Working Tree

### 1. Licensing is now complete

- root [LICENSE](../LICENSE) exists
- [crates/purifier-core/Cargo.toml](../crates/purifier-core/Cargo.toml) declares `license = "MIT"`
- [crates/purifier-tui/Cargo.toml](../crates/purifier-tui/Cargo.toml) declares `license = "MIT"`
- [README.md](../README.md) license text matches the actual license file

### 2. Public artifact and metadata leaks have been reduced

- the README screenshot has been replaced with a sanitized one under [docs/readme/tui-screenshot.png](readme/tui-screenshot.png)
- tracked local-path metadata such as `skills-lock.json` is being removed from the public repo surface
- maintainer-only tooling and reference artifacts such as `.claude/**`, `docs/worktree-*.diff`, and `docs/superpowers/**` are being removed from the public repo surface
- repository scope decisions are documented in [docs/repo_scope.md](repo_scope.md)

### 3. Secret storage and LLM privacy wording is now truthful

- [README.md](../README.md) documents plaintext `secrets.toml` storage and exact path metadata disclosure
- [PROGRESS.md](../PROGRESS.md) reflects the current storage and LLM behavior
- the TUI now shows the same disclosure in onboarding and settings
- future secure storage work is tracked in [docs/ISSUES.md](ISSUES.md)

### 4. OSS contributor and security intake is now present

Added repository surfaces:

- [CONTRIBUTING.md](../CONTRIBUTING.md)
- [SECURITY.md](../.github/SECURITY.md)
- [CODE_OF_CONDUCT.md](../.github/CODE_OF_CONDUCT.md)
- [bug_report.yml](../.github/ISSUE_TEMPLATE/bug_report.yml)
- [docs_or_rules_mismatch.yml](../.github/ISSUE_TEMPLATE/docs_or_rules_mismatch.yml)
- [improvement_request.yml](../.github/ISSUE_TEMPLATE/improvement_request.yml)
- [PULL_REQUEST_TEMPLATE.md](../.github/PULL_REQUEST_TEMPLATE.md)

## Remaining Work Before Public Launch

### 1. Add CI for required verification commands

Why this matters:

- public contributors need fast feedback on whether a change meets the project baseline
- the repo already has a de facto verification contract; CI should enforce it consistently

Target checks:

- `cargo test -p purifier-core`
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings`

### 2. Do one final publication scrub

Why this matters:

- this is the last chance to catch stale private paths, noisy tracked files, or mismatched public-facing copy
- the repo has undergone several cleanup moves, so a final outsider-style review is still worthwhile

Suggested review pass:

- inspect the final tracked file list and root layout
- check screenshots and docs for private names or stale behavior claims
- read the repo as a first-time outside contributor would

### 3. Replace plaintext provider secret storage later

Current state:

- plaintext `secrets.toml` is intentionally the current reliability-first implementation
- this is acceptable only because the app and docs now describe it honestly

Follow-up:

- keep this deferred for now
- track the replacement work in [docs/ISSUES.md](ISSUES.md) issue `#1`

## Review Notes

### Secret scan

- a regex-based scan did not find obvious real committed API keys, private keys, or tokens
- test-only fake keys are present in tests, which is acceptable

### Deletion safety

- the current deletion path appears careful around symlinked directories
- existing tests indicate symlinked directories are not followed during deletion

### Verification status during assessment

The repository checks passed on the reviewed working tree:

- `cargo test -p purifier-core`
- `cargo test -p purifier-tui`
- `cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings`

Note:

- some tests require binding a local test server, so they fail inside a restrictive sandbox but pass when run normally

## Suggested Next Order

1. add CI
2. do the final publication scrub
3. publish with truthful plaintext-storage docs if needed
4. replace plaintext secret storage later

## Bottom Line

Purifier is much closer to open-source-ready now. The original blockers around licensing, repo noise, and misleading docs have been addressed in the current working tree. The main near-term gap is CI, followed by a final publication scrub. Reliable secure secret storage remains important, but it is now a tracked follow-up rather than hidden behavior.
