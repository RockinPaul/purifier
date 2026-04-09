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
