# Security Policy

## Reporting a vulnerability

Please do not report security issues in public GitHub issues.

Use GitHub's private vulnerability reporting flow for this repository if it is enabled. If private reporting is unavailable, contact the maintainers privately through the repository hosting platform before making any disclosure public.

Include:

- affected version or commit
- reproduction steps
- impact assessment
- whether the issue can cause unintended deletion, secret exposure, or path disclosure

## What counts as security-relevant here

For Purifier, the following should be treated as security issues:

- unintended deletion outside the selected target set
- symlink handling that can escape the intended deletion boundary
- exposure of API keys or persisted credentials
- disclosure of local filesystem metadata to remote providers beyond documented behavior
- bypasses that misrepresent safety classification in a way that could cause unsafe deletion

## Supported versions

Purifier is pre-1.0. At the moment, only the latest main branch state is considered supported for security fixes.
