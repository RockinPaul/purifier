# Issues

## 1. Reliable secure provider secret storage

Current state:

- saved provider API keys are stored in plaintext `secrets.toml`
- Unix permissions are restricted to `0600`, but the file is still not encrypted at rest
- this file-based store exists because the previous macOS keychain approach was not reliable across process restarts

Why this matters:

- open-source users should get a storage path that is both reliable and safer than plaintext-on-disk secrets
- the current behavior is acceptable only as a documented interim state

Not today:

- do not block the current release on reintroducing keychain-backed storage
- keep the app truthful about `secrets.toml` until a stable replacement exists

Desired outcome:

- a reliable secret-store abstraction that uses safer platform storage where available
- documented migration from existing `secrets.toml`
- tests covering restart persistence and failure handling

## 2. LLM assessment can remain stuck indefinitely

Current state:

- some files remain at `Analyzing with LLM...` with no eventual success or failure state
- this happens when provider results are partial or path-misaligned relative to the requested batch
- the current behavior is accepted for now because the app does not safely reconcile every partial result

Why this matters:

- users cannot tell whether assessment is still in progress or silently stranded
- some files remain permanently unassessed even though the scan otherwise completed

Not today:

- do not replace the stuck state with something more misleading unless result reconciliation is safe
- keep the app and docs truthful that LLM assessment is best-effort today

Desired outcome:

- reconcile provider responses against the original request batch before applying results
- mark unresolved rows explicitly instead of leaving them in a permanent in-progress state
- make it clear in the UI whether safety came from rules-only or from an LLM assessment

## 3. List scrolling does not follow selection at the viewport edge

Current state:

- when the user presses up or down through a long list, selection moves but the viewport does not always move with it
- once the visible edge is reached, the highlighted row can move out of view instead of scrolling the list

Why this matters:

- keyboard navigation becomes unreliable in long directories
- the user can lose visual context about which row is currently selected

Not today:

- do not paper over this with alternate navigation behavior that changes the Miller Columns model
- keep selection and scrolling behavior truthful to the actual focused row

Desired outcome:

- the visible window should scroll as soon as the selection hits the top or bottom edge
- the selected row should remain visible during continuous `j`/`k` or arrow-key navigation
- scrolling behavior should be consistent across parent and current list panes
