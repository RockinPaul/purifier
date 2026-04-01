# Purifier

A terminal-based disk space analyzer and cleanup tool for macOS, inspired by [GrandPerspective](https://grandperspectiv.sourceforge.io/). Built with Rust and [Ratatui](https://ratatui.rs/).

Purifier scans your filesystem, classifies every path by safety level (Safe / Caution / Unsafe), and explains *why* — so you know what's safe to delete before you hit confirm.

## Features

- **Parallel filesystem scanning** — powered by [jwalk](https://github.com/Byron/jwalk-rs)
- **Physical disk usage by default** — allocated blocks are used for totals, with a logical-size toggle available in settings
- **Hard-link-aware accounting** — physical totals count shared storage once instead of once per pathname
- **Responsive blocking scan UI** — scan progress stays visible and `q` / `Esc` remain responsive while the tree stays hidden until completion
- **Scan profiles** — choose between built-in profiles like `Full scan` and `Fast developer scan`
- **Hybrid safety classification** — built-in TOML rules for common macOS paths + optional LLM (via [OpenRouter](https://openrouter.ai/) or OpenAI) for unknown paths
- **4 tabbed views** — browse by Size, Type, Safety, or Age
- **Interactive deletion** — confirmation shows logical size and estimated physical free space; status bar keeps logical removed bytes and physical freed space separate
- **Vim-style navigation** — `j`/`k`/`h`/`l`, arrow keys, Enter to expand/collapse

## Installation

```bash
# Clone and build
git clone https://github.com/RockinPaul/purifier.git
cd purifier
cargo build --release

# Binary is at target/release/purifier
```

## Usage

```bash
purifier                         # scan full disk (/)
purifier ~/Downloads             # scan specific path
purifier --no-llm ~/Projects     # skip LLM, rules-only classification
purifier --provider openai --api-key YOUR_KEY
```

### Environment variables

| Variable | Description |
|----------|-------------|
| `OPENROUTER_API_KEY` | OpenRouter API key for LLM classification of unknown paths |
| `OPENAI_API_KEY` | OpenAI API key for LLM classification of unknown paths |

### CLI options

| Option | Description |
|--------|-------------|
| `[PATH]` | Directory to scan (defaults to `/`) |
| `--rules <FILE>` | Additional TOML rules file (evaluated before defaults) |
| `--no-llm` | Disable LLM classification entirely |
| `--api-key <KEY>` | API key for the selected remote provider (for example OpenRouter or OpenAI) |
| `--provider <PROVIDER>` | LLM provider. Runtime-wired options are `openrouter` and `openai`. |

## Keybindings

| Key | Action |
|-----|--------|
| `1` / `2` / `3` / `4` | Switch view: Size / Type / Safety / Age |
| `j` / `k` or `Down` / `Up` | Navigate entries |
| `Enter` / `l` / `Right` | Expand directory |
| `h` / `Left` | Collapse directory |
| `d` | Delete selected item (with confirmation) |
| `y` / `n` | Confirm / cancel deletion |
| `q` / `Esc` | Quit |

Inside the settings modal:

- `m` toggles between `Physical` and `Logical` size mode
- `p` cycles the saved scan profile

## Views

### Tab 1: By Size (default)
Directory tree sorted by the currently selected size mode, largest first. Each entry shows a proportional size bar and a colored safety badge.

### Tab 2: By Type
Entries grouped by category: Build Artifacts, Caches, Downloads, App Data, Media, System, Unknown.

### Tab 3: By Safety
The "suggested cleanup" view. Groups entries into Safe (green), Caution (yellow), Unsafe (red), and Unknown (gray).

### Tab 4: By Age
Sorted by last modified date. Highlights old files you may have forgotten about.

## Safety classification

### Built-in rules
Purifier ships with ~30 macOS-specific rules in `rules/default.toml` covering:

| Category | Examples | Default safety |
|----------|----------|----------------|
| **Build Artifacts** | `node_modules/`, `target/debug`, `DerivedData/`, `Pods/`, `__pycache__/` | Safe |
| **Caches** | `~/Library/Caches/*`, `~/.cache/*`, `~/.npm/_cacache`, `~/.Trash/*` | Safe |
| **Downloads** | `~/Downloads/*.dmg`, `~/Downloads/*.pkg`, `~/Downloads/*.zip` | Caution |
| **App Data** | `~/Library/Application Support/*`, `~/Library/Preferences/*` | Unsafe |
| **System** | `.git/objects`, `/System/**`, `~/Library/Keychains/*` | Unsafe |

Rules are evaluated top-to-bottom, first match wins. Add your own rules via `--rules my-rules.toml`.

### LLM fallback
Paths not matched by any rule are batched (up to 50 at a time) and sent to the configured LLM provider for classification. OpenRouter and OpenAI use chat completions APIs. The LLM returns a category, safety level, and one-line explanation for each path.

If the LLM is unavailable, unmatched paths stay "Unknown" — the tool is fully usable without an API key.

## Size semantics

Purifier tracks two size measurements:

- **Physical** — allocated bytes on disk, based on filesystem block allocation
- **Logical** — file content length, closer to what Finder usually shows

Physical size is the default mode in the TUI. Logical size is still available from settings.

Important notes:

- Physical totals are **hard-link-aware** and count shared storage once.
- Per-path delete feedback prefers the **observed volume free-space delta** when available, and falls back to an estimate otherwise.
- APFS clone/shared-block accounting is still **best-effort**. Purifier can see allocated blocks and hard links, but it does not compute exact unique physical ownership for APFS clones or snapshots.

## Scan model

- Scans are currently **blocking**: the progress popup stays visible and the directory tree stays hidden until the scan completes.
- The scan UI remains responsive to `q` / `Esc`.
- LLM batching for unknown paths can overlap with the hidden scan phase, but rows only become visible after the final tree is ready.

## Scan profiles

Purifier supports GrandPerspective-style scan profiles. The current UI lets you select a persisted profile in settings, and the scanner uses the profile's **exclude** filter during traversal.

Built-in profiles:

- `Full scan` — no exclusions
- `Fast developer scan` — excludes common heavyweight developer artifacts such as `node_modules`, `target`, and `DerivedData`

The core filter engine supports tests based on:

- name
- path glob
- size bounds
- file type
- hard-link status
- package status

At the moment, the shipped UI focuses on profile selection and scan-time exclusion. More advanced mask/display-filter controls are not exposed yet.

## Custom rules

Create a TOML file with your own rules:

```toml
[[rules]]
pattern = "**/my-project/dist"
category = "BuildArtifact"
safety = "Safe"
reason = "Build output — regenerated by `npm run build`"

[[rules]]
pattern = "~/Movies/*.mov"
category = "Media"
safety = "Caution"
reason = "Video file — review before deleting"
```

Then run with: `purifier --rules my-rules.toml`

Custom rules are evaluated **before** the defaults, so they take priority.

## Architecture

```
purifier/
├── crates/
│   ├── purifier-core/       # library: scanner, filters, size model, classifier, LLM client
│   └── purifier-tui/        # binary: Ratatui frontend, config, event loop
└── rules/
    └── default.toml         # built-in macOS safety rules
```

Two-crate workspace:
- **purifier-core** — all logic with no terminal dependency. Testable in isolation.
- **purifier-tui** — thin Ratatui shell that renders state and handles input.

### Key dependencies

| Crate | Purpose |
|-------|---------|
| [ratatui](https://ratatui.rs/) | Terminal UI rendering |
| [crossterm](https://github.com/crossterm-rs/crossterm) | Terminal input/output |
| [jwalk](https://github.com/Byron/jwalk-rs) | Parallel filesystem walking |
| [crossbeam-channel](https://github.com/crossbeam-rs/crossbeam) | Scanner-to-TUI message passing |
| [reqwest](https://github.com/seanmonstar/reqwest) | HTTP client for supported LLM providers |
| [clap](https://github.com/clap-rs/clap) | CLI argument parsing |
| [glob](https://github.com/rust-lang/glob) | Pattern matching for rules |
| [keyring](https://github.com/open-source-cooperative/keyring-rs) | macOS Keychain integration |

## Development

```bash
cargo build            # debug build
cargo test             # run all tests
cargo run -- ~/tmp     # quick test scan
cargo clippy --all-targets --all-features --locked -- -D warnings
```

## Platform support

macOS only for now. The safety rules and path conventions are macOS-specific. Linux/Windows support is a future goal — the core architecture is platform-agnostic, only the rules need adapting.

## Similar tools

| Tool | Language | Difference |
|------|----------|------------|
| [dua-cli](https://github.com/Byron/dua-cli) | Rust | No safety classification or explanations |
| [mcdu](https://github.com/mikalv/mcdu) | Rust | Developer-focused, no LLM, no general-user mode |
| [dust](https://github.com/bootandy/dust) | Rust | Non-interactive, no deletion |
| [ncdu](https://code.blicky.net/yorhel/ncdu) | C | No safety classification |
| [GrandPerspective](https://grandperspectiv.sourceforge.io/) | Obj-C | GUI only, no safety classification |

## License

MIT
