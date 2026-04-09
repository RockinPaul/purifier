## Summary

- describe the change
- describe the user-visible effect

## Verification

- [ ] `cargo test -p purifier-core`
- [ ] `cargo test -p purifier-tui`
- [ ] `cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings`

## Docs and product truth

- [ ] I updated `README.md`, `PROGRESS.md`, and `AGENTS.md` if provider/runtime behavior changed.
- [ ] I called out any privacy, credential-storage, or deletion-safety impact.
- [ ] I sanitized screenshots and example paths.
