# Repository Scope For Open Source

This file records what should stay in the public Purifier repository and what should move out as maintainer-only tooling.

## Keep In The Public Repo

- product code under `crates/`
- shipped rules under `rules/`
- user-facing docs and release-facing metadata:
  - `README.md`
  - `PROGRESS.md`
  - `AGENTS.md`
  - `CONTRIBUTING.md`
  - `LICENSE`
  - `.github/**`
  - `docs/ISSUES.md`
  - `docs/readme/**`
- active project notes that directly explain current product behavior:
  - `docs/plan_oss.md`

Why:

- these files help people build, use, review, and contribute to Purifier
- they describe actual product behavior or current release work

## Remove From The Public Repo

- `.claude/**`
- `skills-lock.json`
- `docs/worktree-*.diff`
- `docs/superpowers/**`
- `docs/plans/**`
- `docs/lagging_issue_explanation.md`

Why:

- they are maintainer or agent-tooling artifacts, not part of Purifier itself
- they increase review noise and license surface
- some contain stale design history or environment-specific workflow details
