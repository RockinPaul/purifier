# Plan 04: Right-Aligned Labels, Long Title Animation, UI Legend, and Extended Safety Rules

## Context

The current Miller Columns UI has three UX issues visible in the screenshot:

1. **Misaligned labels**: Safety badges and sizes follow directly after the file name as inline spans, so they're vertically unaligned across rows. "finalize.romfs ⚠ 15.1 MB" and "IMG20250124160550.jpg ⚠ 12.3 MB" have their badges and sizes at different horizontal positions.

2. **Long title truncation**: Long filenames like `Артоболевская_А_Д_Первая_встреча_с_музыкой_Учебное_пособие_1986.pdf` get hard-clipped at the column edge with no indication they're cut off. The user can't read the full name.

3. **Missing legend**: The status bar has minimal cryptic hints (`,.:settings q:quit h/l:nav d:delete`) but there's no discoverable legend explaining how to change sorting, mark files, enter directories, etc.

## Current Architecture

- **`columns_view.rs`**: `render_current_column()` builds each row as `Line::from(vec![mark_span, name_span, badge_span, size_span])` — all spans are concatenated left-to-right with no padding.
- **`columns_view.rs`**: `render_parent_column()` builds each row as `Line::from(vec![name_span, size_span])`.
- **`ui/mod.rs`**: `MillerLayout` splits: sort_indicator(1 row) | columns_area | status_bar(1 row).
- **`status_bar.rs`**: Three-section horizontal bar: breadcrumb | marks/scan | sort+LLM+help.
- **`input.rs`**: All keybindings defined but not discoverable in the UI.
- **`app.rs`**: No state for scroll animation (tick counter, scroll offset).

---

## Fix 1: Right-Aligned Size and Safety Labels

**Goal**: In both parent and current column panes, the filename is left-aligned and the `[badge] [size]` block is right-aligned, with the space between filled by dots or spaces.

**Files**: `crates/purifier-tui/src/ui/columns_view.rs`

### Approach

Each row has a fixed width (`area.width`). We know exactly how many characters the left side (mark + name) and right side (badge + size) take. Fill the gap between them.

**Row layout for current column:**
```
| mark(3) | name(variable) | fill(...) | badge(3) | size(≤9) | pad(1) |
```

**Row layout for parent column:**
```
| pad(1) | name(variable) | fill(...) | size(≤9) | pad(1) |
```

### Implementation

Add a helper function to build a padded row:

```rust
/// Build a single row line with left-aligned name and right-aligned metadata.
/// `left_prefix` is the mark indicator (e.g. " ✘ " or "   ").
/// `name` is the file/directory name.
/// `right_suffix` is the badge+size string (e.g. " ⚠  12.3 MB ").
/// `width` is the total available column width.
/// Returns a Line with padding between name and suffix.
fn padded_row(
    left_prefix: &str,
    name: &str,
    right_suffix: &str,
    width: u16,
    prefix_style: Style,
    name_style: Style,
    suffix_style: Style,
) -> Line<'static> {
    let prefix_w = unicode_width(left_prefix);
    let suffix_w = unicode_width(right_suffix);
    let name_w = unicode_width(name);
    let total_content = prefix_w + name_w + suffix_w;
    let available = width as usize;

    if total_content >= available {
        // Truncate name to fit
        let max_name = available.saturating_sub(prefix_w + suffix_w);
        let truncated = truncate_to_width(name, max_name);
        Line::from(vec![
            Span::styled(left_prefix.to_string(), prefix_style),
            Span::styled(truncated, name_style),
            Span::styled(right_suffix.to_string(), suffix_style),
        ])
    } else {
        let gap = available - total_content;
        let fill = " ".repeat(gap);
        Line::from(vec![
            Span::styled(left_prefix.to_string(), prefix_style),
            Span::styled(name.to_string(), name_style),
            Span::styled(fill, Style::default()),
            Span::styled(right_suffix.to_string(), suffix_style),
        ])
    }
}
```

Update `render_current_column()`:
- Build `right_suffix` as `format!(" {} {:>8} ", safety_badge(entry.safety), size_str)` — fixed-width size field with right-padding.
- Build `left_prefix` as mark indicator string.
- Call `padded_row(prefix, &name, &right_suffix, area.width, ...)`.

Update `render_parent_column()`:
- Build `right_suffix` as `format!(" {:>8} ", size_str)`.
- Call `padded_row(" ", &name, &right_suffix, area.width, ...)`.

### Unicode Width

Use `unicode_width` crate (or inline char-width estimation) for correct handling of CJK and emoji characters. The Cargo.toml for `purifier-tui` should add `unicode-width = "0.2"`.

Alternatively, count character widths manually: ASCII = 1, CJK/emoji = 2. This avoids a dependency.

**Simpler approach**: Use `str.chars().count()` as an approximation since ratatui's Span rendering counts characters. For V1, this is sufficient. If CJK alignment is off, add `unicode-width` later.

### Fixed-Width Size Column

Format all sizes to a fixed width to ensure vertical alignment:

```rust
fn format_size_fixed(bytes: u64) -> String {
    // Returns a right-aligned string of exactly 8 chars, e.g. " 12.3 MB"
    let s = format_size(bytes);
    format!("{:>8}", s)
}
```

### Size Badge Layout

The right-aligned block is always the same width: `" ⚠ " + "  12.3 MB "` = badge(3) + size(10) + pad(1) = 14 chars. This ensures vertical alignment across all rows.

---

## Fix 2: Horizontal Scroll Animation for Long Titles

**Goal**: When a filename is too long to fit in the available space, the selected row should smoothly scroll (marquee-style) to reveal the full name. Non-selected long names show a truncated version with an ellipsis.

### State Changes

**File**: `crates/purifier-tui/src/app.rs`

Add to `App`:
```rust
pub name_scroll_offset: usize,    // Character offset for selected row's name scroll
pub name_scroll_tick: u16,        // Frame counter for scroll timing
pub name_scroll_pause: u16,       // Pause frames at start/end before scrolling
```

Constants:
```rust
const NAME_SCROLL_SPEED: u16 = 4;    // Advance 1 char every N frames (at 60fps = ~4 chars/sec)
const NAME_SCROLL_PAUSE: u16 = 60;   // Pause 1 second at start and end
```

### Scroll Logic

**File**: `crates/purifier-tui/src/app.rs`

Add method `App::advance_name_scroll(&mut self, max_name_width: usize)`:
```rust
pub fn advance_name_scroll(&mut self) {
    self.name_scroll_tick += 1;
}
```

The actual scroll offset computation happens during render, based on the tick counter and the overflow amount:

```rust
fn compute_scroll_offset(tick: u16, overflow: usize, speed: u16, pause: u16) -> usize {
    if overflow == 0 { return 0; }
    let total_scroll_ticks = overflow as u16 * speed;
    let cycle = pause + total_scroll_ticks + pause + total_scroll_ticks;
    let pos = tick % cycle;

    if pos < pause {
        0  // Paused at start
    } else if pos < pause + total_scroll_ticks {
        ((pos - pause) / speed) as usize  // Scrolling right
    } else if pos < pause + total_scroll_ticks + pause {
        overflow  // Paused at end
    } else {
        overflow - ((pos - pause - total_scroll_ticks - pause) / speed) as usize  // Scrolling back
    }
}
```

### Invalidation

When selection changes (j/k/h/l/Enter), reset:
```rust
app.name_scroll_offset = 0;
app.name_scroll_tick = 0;
```

Add this reset to `invalidate_preview_cache()` since it's called on every selection change.

### Render Integration

**File**: `crates/purifier-tui/src/ui/columns_view.rs`

In `render_current_column()`, for the selected row only:
- Compute `overflow = name_width - max_name_width` (if name is longer than available space).
- Compute `scroll_offset = compute_scroll_offset(app.name_scroll_tick, overflow, SPEED, PAUSE)`.
- Slice the name string starting at `scroll_offset` characters.
- For non-selected rows, truncate with trailing `…` (ellipsis).

### Tick Advance

**File**: `crates/purifier-tui/src/main.rs`

In the event loop, right before `terminal.draw()`, call:
```rust
app.advance_name_scroll();
```

This increments the tick counter every frame (16ms at 60fps). The scroll offset is computed from the tick during render.

### Non-Selected Truncation

For non-selected rows that overflow, show:
```
"very_long_filename_that_does_not_f…"
```

Use a `truncate_with_ellipsis(name, max_width)` helper:
```rust
fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", truncated)
}
```

---

## Fix 3: UI Help Legend

**Goal**: Make keybindings discoverable. Two approaches combined:

### 3a: Expanded Status Bar Help

**File**: `crates/purifier-tui/src/ui/status_bar.rs`

Replace the terse ` | ,:settings q:quit h/l:nav d:delete ` with a more readable format:

```
 j/k:↑↓  h/l:←→  Enter:open  d:delete  Space:mark  x:batch  s:sort  ,:settings  q:quit
```

Group by action category and use Unicode arrows for clarity.

### 3b: Help Overlay (? key)

**File**: `crates/purifier-tui/src/ui/help_overlay.rs` (NEW)
**File**: `crates/purifier-tui/src/input.rs`

Add a `?` keybinding that toggles a help overlay (similar to the scanning overlay):

```
┌─ Help ─────────────────────────────┐
│                                    │
│  Navigation                        │
│    j / ↓       Move down           │
│    k / ↑       Move up             │
│    h / ←       Go to parent        │
│    l / → / ⏎   Enter directory     │
│    g           Jump to top         │
│    G           Jump to bottom      │
│    ~           Go to home          │
│                                    │
│  Actions                           │
│    d           Delete selected     │
│    Space       Mark/unmark         │
│    x           Review batch        │
│    u           Clear all marks     │
│                                    │
│  View                              │
│    s           Cycle sort order    │
│    i           Toggle size mode    │
│                                    │
│  Other                             │
│    ,           Open settings       │
│    ?           Toggle this help    │
│    q / Esc     Quit                │
│                                    │
│  Press ? or Esc to close           │
└────────────────────────────────────┘
```

### State Changes

**File**: `crates/purifier-tui/src/app.rs`

Add `PreviewMode::Help` variant:
```rust
pub enum PreviewMode {
    Analytics,
    DeleteConfirm(PathBuf),
    BatchReview,
    Settings(SettingsDraft),
    Onboarding(SettingsDraft),
    Help,  // NEW
}
```

**File**: `crates/purifier-tui/src/input.rs`

In `handle_main_analytics()`, add:
```rust
KeyCode::Char('?') => {
    app.preview_mode = PreviewMode::Help;
}
```

In `handle_main()`, add match arm for `PreviewMode::Help`:
```rust
PreviewMode::Help => {
    match key.code {
        KeyCode::Char('?') | KeyCode::Esc => {
            app.preview_mode = PreviewMode::Analytics;
        }
        _ => {}
    }
    InputResult::None
}
```

**File**: `crates/purifier-tui/src/ui/mod.rs`

In `draw_main()`, after the three panes, check for Help mode and draw overlay on top:
```rust
if matches!(app.preview_mode, PreviewMode::Help) {
    help_overlay::draw(frame, frame.area());
}
```

### Sort Order Legend in Sort Indicator

**File**: `crates/purifier-tui/src/ui/columns_view.rs`

Enhance the sort indicator row to also show the cycling hint:

```
Sort: [Size ▼]  (press s to cycle: Size → Safety → Age → Name)
```

When space is limited, show just:
```
Sort: [Size ▼]  s:cycle
```

---

## Fix 4: Extended Safety Detection Rules

**Goal**: Expand `rules/default.toml` from 32 rules to ~90+ rules covering all major gaps: missing build artifact types, package manager caches, Xcode/developer tooling, credentials/secrets, media paths, IDE caches, macOS system metadata, and Docker/container paths. Also improve the LLM classification prompt with a system message.

### Current State (32 rules)

The existing `default.toml` covers basics well but has significant gaps:
- **Zero rules for Media category** — `~/Movies`, `~/Music`, `~/Pictures` are unclassified
- **Limited build artifacts** — missing `.next`, `dist/`, `.parcel-cache`, `.mypy_cache`, `.pytest_cache`, `.ruff_cache`, `*.egg-info`, `.venv`, `bazel-*`, `.expo`, `.metro`, `cmake-build-*`, `.stack-work`, `dist-newstyle`, `zig-cache`, `.godot`, `.turbo`, `.terraform`
- **Missing package manager caches** — `~/.yarn/cache`, `~/Library/Caches/pip`, `~/.conda/pkgs`, `~/.gradle/caches`, `~/.m2/repository`, `~/.pub-cache`, `~/.gem`, `~/.deno`, `~/.cache/go-build`, `~/.nuget/packages`, `~/.bun/install/cache`, `~/Library/Caches/CocoaPods`, `~/Library/Caches/Homebrew`
- **Missing Xcode paths** — `~/Library/Developer/Xcode/Archives`, `~/Library/Developer/CoreSimulator`, `~/Library/Developer/Xcode/iOS DeviceSupport`
- **Missing credentials** — `~/.ssh/*`, `~/.gnupg/*`, `~/.aws/*`, `~/.kube/*`, `**/.env`, `**/*.pem`, `**/*.key`
- **No IDE caches** — `~/.cache/JetBrains`, `**/.idea/caches`, `**/.vs`
- **No macOS metadata** — `**/.DS_Store`, `**/.Spotlight-V100`, `**/.fseventsd`, `**/.Trashes`, `**/.TemporaryItems`
- **No Docker** — `~/Library/Containers/com.docker.docker`
- **No browser/app-specific data** — Firefox/Chrome profiles
- **Missing system paths** — `/bin/**`, `/sbin/**`, `/private/etc/**`

### New Rules to Add

**File**: `rules/default.toml`

Rules are organized by section. New rules inserted into the appropriate section, maintaining first-match-wins order with more specific patterns before catch-alls.

#### Additional Build Artifacts (Safe) — 25 new rules

```toml
# JavaScript/TypeScript build tools
[[rules]]
pattern = "**/.next"
category = "BuildArtifact"
safety = "Safe"
reason = "Next.js build output — regenerated by `next build`"

[[rules]]
pattern = "**/.nuxt"
category = "BuildArtifact"
safety = "Safe"
reason = "Nuxt.js build output — regenerated by `nuxt build`"

[[rules]]
pattern = "**/.parcel-cache"
category = "BuildArtifact"
safety = "Safe"
reason = "Parcel bundler cache — regenerated on build"

[[rules]]
pattern = "**/.angular"
category = "BuildArtifact"
safety = "Safe"
reason = "Angular CLI cache — regenerated on build"

[[rules]]
pattern = "**/.turbo"
category = "BuildArtifact"
safety = "Safe"
reason = "Turborepo cache — regenerated by `turbo run`"

[[rules]]
pattern = "**/.expo"
category = "BuildArtifact"
safety = "Safe"
reason = "Expo build cache — regenerated by Expo CLI"

[[rules]]
pattern = "**/.metro"
category = "BuildArtifact"
safety = "Safe"
reason = "Metro bundler cache — regenerated on React Native build"

# Python tooling caches
[[rules]]
pattern = "**/.mypy_cache"
category = "BuildArtifact"
safety = "Safe"
reason = "mypy type-check cache — regenerated by `mypy`"

[[rules]]
pattern = "**/.pytest_cache"
category = "BuildArtifact"
safety = "Safe"
reason = "pytest cache — regenerated on test run"

[[rules]]
pattern = "**/.ruff_cache"
category = "BuildArtifact"
safety = "Safe"
reason = "Ruff linter cache — regenerated automatically"

[[rules]]
pattern = "**/.nox"
category = "BuildArtifact"
safety = "Safe"
reason = "nox session environments — regenerated by `nox`"

[[rules]]
pattern = "**/__pypackages__"
category = "BuildArtifact"
safety = "Safe"
reason = "PEP 582 local packages — regenerated on install"

[[rules]]
pattern = "**/*.egg-info"
category = "BuildArtifact"
safety = "Safe"
reason = "Python egg metadata — regenerated by setup.py/pip"

[[rules]]
pattern = "**/.ipynb_checkpoints"
category = "BuildArtifact"
safety = "Safe"
reason = "Jupyter notebook checkpoints — regenerated automatically"

# C/C++ build outputs
[[rules]]
pattern = "**/cmake-build-debug"
category = "BuildArtifact"
safety = "Safe"
reason = "CMake debug build — regenerated by cmake"

[[rules]]
pattern = "**/cmake-build-release"
category = "BuildArtifact"
safety = "Safe"
reason = "CMake release build — regenerated by cmake"

# Other languages
[[rules]]
pattern = "**/.stack-work"
category = "BuildArtifact"
safety = "Safe"
reason = "Haskell Stack build output — regenerated by `stack build`"

[[rules]]
pattern = "**/dist-newstyle"
category = "BuildArtifact"
safety = "Safe"
reason = "Haskell Cabal build output — regenerated by `cabal build`"

[[rules]]
pattern = "**/zig-cache"
category = "BuildArtifact"
safety = "Safe"
reason = "Zig build cache — regenerated by `zig build`"

[[rules]]
pattern = "**/.zig-cache"
category = "BuildArtifact"
safety = "Safe"
reason = "Zig build cache (hidden) — regenerated by `zig build`"

[[rules]]
pattern = "**/zig-out"
category = "BuildArtifact"
safety = "Safe"
reason = "Zig build output — regenerated by `zig build`"

[[rules]]
pattern = "**/.godot"
category = "BuildArtifact"
safety = "Safe"
reason = "Godot 4 import cache — regenerated by editor"

[[rules]]
pattern = "**/.terraform"
category = "BuildArtifact"
safety = "Safe"
reason = "Terraform provider cache — regenerated by `terraform init`"

[[rules]]
pattern = "**/.pixi"
category = "BuildArtifact"
safety = "Safe"
reason = "Pixi package cache — regenerated by pixi"

[[rules]]
pattern = "**/_build"
category = "BuildArtifact"
safety = "Safe"
reason = "Elixir/Erlang build output — regenerated by `mix compile`"
```

#### Additional Caches (Safe) — 18 new rules

```toml
# Package manager download caches
[[rules]]
pattern = "~/.yarn/cache"
category = "Cache"
safety = "Safe"
reason = "Yarn v2+ package cache — re-downloaded as needed"

[[rules]]
pattern = "~/.bun/install/cache"
category = "Cache"
safety = "Safe"
reason = "Bun package cache — re-downloaded on install"

[[rules]]
pattern = "~/Library/Caches/pip"
category = "Cache"
safety = "Safe"
reason = "pip download cache — re-downloaded on install"

[[rules]]
pattern = "~/.cache/pip"
category = "Cache"
safety = "Safe"
reason = "pip download cache (Linux) — re-downloaded on install"

[[rules]]
pattern = "~/.cache/uv"
category = "Cache"
safety = "Safe"
reason = "uv (Python) package cache — re-downloaded on install"

[[rules]]
pattern = "~/.conda/pkgs"
category = "Cache"
safety = "Safe"
reason = "Conda package cache — re-downloaded on install"

[[rules]]
pattern = "~/Library/Caches/Homebrew"
category = "Cache"
safety = "Safe"
reason = "Homebrew download cache — re-downloaded as needed"

[[rules]]
pattern = "~/Library/Caches/CocoaPods"
category = "Cache"
safety = "Safe"
reason = "CocoaPods spec/download cache — re-downloaded on install"

[[rules]]
pattern = "~/.pub-cache"
category = "Cache"
safety = "Safe"
reason = "Dart pub package cache — re-downloaded on install"

[[rules]]
pattern = "~/.gem"
category = "Cache"
safety = "Safe"
reason = "Ruby gem cache — re-downloaded on install"

[[rules]]
pattern = "~/.deno"
category = "Cache"
safety = "Safe"
reason = "Deno cached modules — re-downloaded on import"

[[rules]]
pattern = "~/.gradle/caches"
category = "Cache"
safety = "Safe"
reason = "Gradle download/transform caches — re-downloaded as needed"

[[rules]]
pattern = "~/.m2/repository"
category = "Cache"
safety = "Safe"
reason = "Maven local repository — re-downloaded from remote repos"

[[rules]]
pattern = "~/.cache/go-build"
category = "Cache"
safety = "Safe"
reason = "Go build cache — regenerated by `go build`"

[[rules]]
pattern = "~/.nuget/packages"
category = "Cache"
safety = "Safe"
reason = "NuGet package cache — re-downloaded on restore"

[[rules]]
pattern = "~/.cargo/registry/src"
category = "Cache"
safety = "Safe"
reason = "Cargo extracted crate sources — re-downloaded as needed"

# macOS system caches
[[rules]]
pattern = "~/Library/Saved Application State/*"
category = "Cache"
safety = "Safe"
reason = "Window restore state — apps recreate on launch"

[[rules]]
pattern = "/private/var/folders/**"
category = "Cache"
safety = "Safe"
reason = "macOS per-user temp — managed by the OS"
```

#### macOS Metadata (Safe) — 5 new rules

```toml
[[rules]]
pattern = "**/.DS_Store"
category = "Cache"
safety = "Safe"
reason = "Finder metadata — tiny, regenerated automatically"

[[rules]]
pattern = "**/.Spotlight-V100"
category = "Cache"
safety = "Safe"
reason = "Spotlight search index — rebuilt automatically"

[[rules]]
pattern = "**/.fseventsd"
category = "Cache"
safety = "Safe"
reason = "FSEvents database — rebuilt automatically"

[[rules]]
pattern = "**/.Trashes"
category = "Cache"
safety = "Safe"
reason = "Volume-level trash — safe to delete"

[[rules]]
pattern = "**/.TemporaryItems"
category = "Cache"
safety = "Safe"
reason = "macOS temporary items — safe to delete"
```

#### IDE Caches (Safe) — 3 new rules

```toml
[[rules]]
pattern = "~/.cache/JetBrains"
category = "Cache"
safety = "Safe"
reason = "JetBrains IDE caches — regenerated on launch"

[[rules]]
pattern = "**/.idea/caches"
category = "Cache"
safety = "Safe"
reason = "IntelliJ project caches — regenerated on open"

[[rules]]
pattern = "**/.vs"
category = "Cache"
safety = "Safe"
reason = "Visual Studio local settings cache — regenerated on open"
```

#### Xcode Developer Paths (Safe/Caution) — 5 new rules

```toml
[[rules]]
pattern = "~/Library/Developer/Xcode/Archives"
category = "BuildArtifact"
safety = "Caution"
reason = "Xcode app archives — safe if already submitted to App Store"

[[rules]]
pattern = "~/Library/Developer/CoreSimulator/Caches"
category = "Cache"
safety = "Safe"
reason = "iOS Simulator caches — regenerated automatically"

[[rules]]
pattern = "~/Library/Developer/CoreSimulator/Devices"
category = "AppData"
safety = "Caution"
reason = "iOS Simulator devices — may contain test data; use `xcrun simctl delete unavailable`"

[[rules]]
pattern = "~/Library/Developer/Xcode/iOS DeviceSupport"
category = "Cache"
safety = "Safe"
reason = "Debug symbols for old iOS versions — re-downloaded when device connects"

[[rules]]
pattern = "~/Library/Developer/Xcode/UserData/Previews"
category = "Cache"
safety = "Safe"
reason = "SwiftUI preview artifacts — regenerated by Xcode"
```

#### Temp/Crash Files (Safe) — 4 new rules

```toml
[[rules]]
pattern = "**/*.core"
category = "System"
safety = "Safe"
reason = "Core dump — safe to delete, only needed for post-mortem debugging"

[[rules]]
pattern = "**/*.crash"
category = "System"
safety = "Safe"
reason = "Crash report — safe to delete"

[[rules]]
pattern = "**/*.ips"
category = "System"
safety = "Safe"
reason = "macOS crash report — safe to delete"

[[rules]]
pattern = "~/Library/Logs/DiagnosticReports/*"
category = "System"
safety = "Safe"
reason = "Diagnostic crash reports — safe to delete"
```

#### Additional Downloads (Caution) — 4 new rules

```toml
[[rules]]
pattern = "~/Downloads/*.iso"
category = "Download"
safety = "Caution"
reason = "Disk image — often large; check if still needed"

[[rules]]
pattern = "~/Downloads/*.7z"
category = "Download"
safety = "Caution"
reason = "Archive — check if contents were extracted"

[[rules]]
pattern = "~/Downloads/*.rar"
category = "Download"
safety = "Caution"
reason = "Archive — check if contents were extracted"

[[rules]]
pattern = "~/Downloads/*.exe"
category = "Download"
safety = "Caution"
reason = "Windows executable on macOS — usually unneeded"
```

#### Media Paths (Caution) — 4 new rules

These fill the completely empty Media category.

```toml
[[rules]]
pattern = "~/Movies/*"
category = "Media"
safety = "Caution"
reason = "User videos — review before deleting"

[[rules]]
pattern = "~/Music/*"
category = "Media"
safety = "Caution"
reason = "User music — review before deleting"

[[rules]]
pattern = "~/Pictures/*"
category = "Media"
safety = "Caution"
reason = "User photos and images — review before deleting"
```

Note: `~/Pictures/Photos Library.photoslibrary` is caught by `~/Pictures/*` as Caution, which is appropriate — the Photos library is massive and user-critical.

#### Docker (Caution) — 2 new rules

```toml
[[rules]]
pattern = "~/Library/Containers/com.docker.docker"
category = "AppData"
safety = "Caution"
reason = "Docker Desktop data — images and volumes; use `docker system prune` instead"

[[rules]]
pattern = "~/.docker/config.json"
category = "System"
safety = "Unsafe"
reason = "Docker registry credentials — may contain auth tokens"
```

#### iOS Backups (Caution) — 1 new rule

```toml
[[rules]]
pattern = "~/Library/Application Support/MobileSync/Backup/*"
category = "AppData"
safety = "Caution"
reason = "iOS device backup — keep latest per device; can be very large"
```

#### Credentials and Secrets (Unsafe) — 12 new rules

These are critical: must be placed **before** the catch-all `~/Library/Application Support/*` rule so they match first.

```toml
[[rules]]
pattern = "~/.ssh/*"
category = "System"
safety = "Unsafe"
reason = "SSH keys — irreplaceable if not backed up"

[[rules]]
pattern = "~/.gnupg/*"
category = "System"
safety = "Unsafe"
reason = "GPG keys and trust database — irreplaceable if not backed up"

[[rules]]
pattern = "~/.aws/*"
category = "System"
safety = "Unsafe"
reason = "AWS credentials and config — contains access keys"

[[rules]]
pattern = "~/.kube/*"
category = "System"
safety = "Unsafe"
reason = "Kubernetes config — contains cluster credentials"

[[rules]]
pattern = "~/.gcloud/*"
category = "System"
safety = "Unsafe"
reason = "Google Cloud credentials — contains service account keys"

[[rules]]
pattern = "~/.azure/*"
category = "System"
safety = "Unsafe"
reason = "Azure credentials — contains authentication tokens"

[[rules]]
pattern = "**/.env"
category = "System"
safety = "Unsafe"
reason = "Environment file — often contains secrets and API keys"

[[rules]]
pattern = "**/.env.local"
category = "System"
safety = "Unsafe"
reason = "Local environment file — often contains secrets"

[[rules]]
pattern = "**/.env.production"
category = "System"
safety = "Unsafe"
reason = "Production environment file — contains production secrets"

[[rules]]
pattern = "**/*.pem"
category = "System"
safety = "Unsafe"
reason = "SSL/TLS certificate or private key — may be irreplaceable"

[[rules]]
pattern = "**/*.key"
category = "System"
safety = "Unsafe"
reason = "Private key file — may be irreplaceable"

[[rules]]
pattern = "~/.config/*"
category = "AppData"
safety = "Unsafe"
reason = "XDG config directory — user preferences and app configuration"
```

#### Additional System Protection (Unsafe) — 5 new rules

```toml
[[rules]]
pattern = "/bin/**"
category = "System"
safety = "Unsafe"
reason = "Essential system binaries — do not delete"

[[rules]]
pattern = "/sbin/**"
category = "System"
safety = "Unsafe"
reason = "System administration binaries — do not delete"

[[rules]]
pattern = "/private/etc/**"
category = "System"
safety = "Unsafe"
reason = "System configuration files — do not delete"

[[rules]]
pattern = "~/Library/Messages"
category = "AppData"
safety = "Unsafe"
reason = "iMessage history — irreplaceable conversation data"

[[rules]]
pattern = "~/Library/Mail/*"
category = "AppData"
safety = "Unsafe"
reason = "Apple Mail data — contains email messages and attachments"
```

#### Application Browser Data (Unsafe) — 2 new rules

```toml
[[rules]]
pattern = "~/Library/Application Support/Firefox/Profiles"
category = "AppData"
safety = "Unsafe"
reason = "Firefox profiles — bookmarks, passwords, browsing data"

[[rules]]
pattern = "~/Library/Application Support/Google/Chrome"
category = "AppData"
safety = "Unsafe"
reason = "Chrome profiles — bookmarks, passwords, extensions"
```

### Improved LLM System Prompt

**File**: `crates/purifier-core/src/llm.rs`

The current LLM prompt is a single `user` message with no system message. Add a `system` message to improve classification reliability:

```rust
fn classification_messages(batch_text: &str) -> Vec<Message> {
    vec![
        Message {
            role: "system".to_string(),
            content: "You are a macOS/Linux filesystem safety classifier for a disk cleanup tool. \
                      Your task is to classify file paths by category and safety level. \
                      Be conservative: when uncertain, prefer Caution over Safe. \
                      Never classify credentials, keys, or git repositories as Safe. \
                      Respond with ONLY valid JSON — no markdown, no explanation.".to_string(),
        },
        Message {
            role: "user".to_string(),
            content: batch_text,
        },
    ]
}
```

### Rule Ordering Strategy

The final `default.toml` must maintain correct first-match-wins ordering:

1. **Credentials/secrets patterns** (`**/.env`, `~/.ssh/*`, etc.) — these must match before any catch-all
2. **Browser/app-specific Unsafe** (`~/Library/Application Support/Firefox/Profiles`, etc.) — before the catch-all `~/Library/Application Support/*`
3. **iOS backup** (`~/Library/Application Support/MobileSync/Backup/*`) — before the catch-all
4. **Build artifacts** (most specific patterns)
5. **Caches** (specific → catch-all)
6. **Downloads** (specific extensions → catch-all)
7. **Media** (user media folders)
8. **App Data catch-alls** (`~/Library/Application Support/*`, etc.)
9. **System** (`/System/**`, `/usr/**`, etc.)
10. **User folders** (`~/Documents/*`, `~/Desktop/*`)

### Summary

| Category | Before | After |
|----------|--------|-------|
| Build Artifacts | 12 rules | 37 rules |
| Cache (Safe) | 6 rules | 29 rules |
| Download (Caution) | 5 rules | 9 rules |
| Media (Caution) | 0 rules | 3 rules |
| App Data (Unsafe/Caution) | 3+2 rules | 10+ rules |
| System (Unsafe) | 5 rules | 17 rules |
| Credentials (Unsafe) | 0 rules | 12 rules |
| **Total** | **32 rules** | **~90 rules** |

### Edge Cases Documented

Add comments in `default.toml` for tricky cases:

```toml
# EDGE CASE: pnpm store uses hard links — deleting store breaks node_modules.
# Use `pnpm store prune` instead of manual deletion. Classified as Caution.

# EDGE CASE: ~/.conda/envs contains active virtualenvs (Unsafe),
# but ~/.conda/pkgs is a download cache (Safe). Different safety levels.

# EDGE CASE: ~/Library/Application Support subdirs for uninstalled apps are
# orphaned data (could be Caution), but we can't detect app installation status
# from path alone. Default to Unsafe, let LLM refine.

# EDGE CASE: .git directories may contain uncommitted work, stashes, and
# unpushed branches. Always Unsafe — there is no manifest to regenerate from.
```

---

## Fix 5: Mouse and Trackpad Selection and Navigation

**Goal**: Enable full mouse/trackpad interaction in the Miller Columns view — click to select, double-click to enter, right-click to go back, scroll in any pane, and horizontal trackpad swipe for navigation. This matches the UX conventions of ranger, lf, and yazi.

### Current State

Mouse capture is already enabled (`EnableMouseCapture` in `main.rs:195`). The existing `handle_mouse` function (`input.rs:30`) only handles `ScrollDown`/`ScrollUp` to move selection by 1 in the current column. Click coordinates (`mouse.column`, `mouse.row`) are completely ignored. There is no pane hit-testing, no click-to-select, and no double-click detection.

**Bug in existing handler**: Mouse scroll does not call `app.invalidate_preview_cache()`, unlike the equivalent j/k keyboard handlers. This means the preview pane goes stale after scrolling.

### Architecture: Store Layout for Hit-Testing

The `MillerLayout` is computed during `draw()` but mouse events arrive in the event loop. The layout must be persisted so the mouse handler can map coordinates to panes.

**File**: `crates/purifier-tui/src/app.rs`

Add fields:
```rust
pub last_layout: Option<MillerLayout>,
pub last_click: Option<(u16, u16, std::time::Instant)>,  // (col, row, time) for double-click detection
```

**File**: `crates/purifier-tui/src/ui/mod.rs`

In `draw_main()`, after computing the layout, store it on `app`:
```rust
fn draw_main(frame: &mut Frame, app: &mut App) {   // NOTE: &mut App, not &App
    let has_parent = app.columns.parent().is_some();
    let layout = miller_layout(frame.area(), has_parent);
    app.last_layout = Some(layout);
    // ... rest of draw
}
```

This requires changing the `draw` signature from `&App` to `&mut App`. Since `terminal.draw()` takes an `FnOnce(&mut Frame)`, we need to pass `app` into the closure. The current code already does `terminal.draw(|frame| ui::draw(frame, &app))?;` — change to `&mut app`.

**Alternative (simpler)**: Recompute the layout in the mouse handler using `miller_layout(terminal_size, has_parent)`. Since `miller_layout` is pure and cheap, this avoids the `&mut` signature change entirely. Store the terminal size on `App` instead:

```rust
pub terminal_size: Rect,  // Updated each frame before draw
```

In `main.rs` before draw:
```rust
app.terminal_size = terminal.size()?;
```

Then in the mouse handler:
```rust
let has_parent = app.columns.parent().is_some();
let layout = miller_layout(app.terminal_size, has_parent);
```

This approach is cleaner — no signature changes needed.

### Gesture Mapping

Based on ranger/lf/yazi conventions:

| Gesture | Current Column | Parent Column | Preview Pane | Sort Indicator | Status Bar |
|---------|---------------|---------------|--------------|----------------|------------|
| Left click | Select row | Navigate back + select clicked entry | Enter directory (if dir selected) | No-op | No-op |
| Double-click | Enter directory / toggle mark on file | Enter clicked entry | No-op | No-op | No-op |
| Right click | Go back to parent | Go back another level | No-op | No-op | No-op |
| Scroll up/down | Move selection up/down | Move selection in parent | Scroll preview (future) | No-op | No-op |
| ScrollLeft | Go back to parent | — | — | — | — |
| ScrollRight | Enter selected directory | — | — | — | — |
| Middle click | Toggle mark (= Space) | No-op | No-op | No-op | No-op |

### Gesture Reference in Settings Screen

The gesture mapping must be displayed in the Settings screen (`render_settings` in `preview_pane.rs`) after all existing settings (Scan Profile row) and before the LLM status indicator. This serves as in-app documentation so the user can discover mouse interactions.

**File**: `crates/purifier-tui/src/ui/preview_pane.rs`

In `render_settings()`, after the Scan Profile row (line 493) and before the blank line + LLM status (line 495), insert a new section. Only show it in Settings mode, not during Onboarding (onboarding should stay focused on setup):

```rust
    // Gesture mapping reference (settings only, not onboarding)
    if !is_onboarding {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Mouse & Trackpad",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("    Left click      ", Style::default().fg(Color::DarkGray)),
            Span::raw("Select entry"),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    Double-click    ", Style::default().fg(Color::DarkGray)),
            Span::raw("Open dir / mark file"),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    Right click     ", Style::default().fg(Color::DarkGray)),
            Span::raw("Go back"),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    Middle click    ", Style::default().fg(Color::DarkGray)),
            Span::raw("Toggle mark"),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    Scroll ↑/↓      ", Style::default().fg(Color::DarkGray)),
            Span::raw("Move selection"),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    Swipe ←/→       ", Style::default().fg(Color::DarkGray)),
            Span::raw("Navigate back/forward"),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    Click parent    ", Style::default().fg(Color::DarkGray)),
            Span::raw("Go to parent dir"),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    Click preview   ", Style::default().fg(Color::DarkGray)),
            Span::raw("Enter selected dir"),
        ]));
    }
```

**Resulting Settings layout order:**

1. Header ("Settings")
2. Provider selection (1-4 keys)
3. API Key
4. Model (read-only)
5. Size Mode (Physical/Logical)
6. Scan Profile
7. **Mouse & Trackpad reference** (NEW — settings only)
8. LLM status indicator
9. Error/saving messages
10. Footer (Enter/Esc)

The gesture reference uses the same visual style as the rest of settings: left column in DarkGray for the gesture name (fixed-width for alignment), right column in default color for the action description.

---

### Hit-Testing: Mapping Coordinates to Rows

Each column pane has a 1-row header for the directory name. The list content starts at `pane.y + 1`.

```rust
fn click_to_row(pane: Rect, mouse_row: u16, scroll_offset: usize) -> Option<usize> {
    let header_height = 1u16;
    let list_start_y = pane.y + header_height;
    if mouse_row < list_start_y || mouse_row >= pane.y + pane.height {
        return None;  // Click is in the header or outside the pane
    }
    let visual_row = (mouse_row - list_start_y) as usize;
    Some(scroll_offset + visual_row)
}
```

The returned value is an index into the sorted list (same space as `col.selected_index`).

### Double-Click Detection

Crossterm has no built-in double-click event. Detect it by timing consecutive `Down(Left)` events:

```rust
const DOUBLE_CLICK_MS: u128 = 400;

fn is_double_click(app: &App, col: u16, row: u16) -> bool {
    if let Some((prev_col, prev_row, prev_time)) = app.last_click {
        let elapsed = prev_time.elapsed().as_millis();
        elapsed < DOUBLE_CLICK_MS && prev_col == col && prev_row == row
    } else {
        false
    }
}
```

### Implementation

**File**: `crates/purifier-tui/src/input.rs`

Replace the current `handle_mouse` with a full implementation:

```rust
pub fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    if !matches!(app.screen, AppScreen::Main) {
        return;
    }
    if !matches!(app.preview_mode, PreviewMode::Analytics) {
        return;
    }
    if matches!(app.scan_status, ScanStatus::Scanning) {
        return;  // No interaction during scan
    }

    let has_parent = app.columns.parent().is_some();
    let layout = miller_layout(app.terminal_size, has_parent);
    let pos = Position { x: mouse.column, y: mouse.row };

    match mouse.kind {
        // -- Scroll wheel --
        MouseEventKind::ScrollDown => {
            if layout.current_column.contains(pos) || layout.parent_column.contains(pos) {
                let count = app.current_children_count();
                app.columns.move_selection(1, count);
                app.invalidate_preview_cache();
            }
        }
        MouseEventKind::ScrollUp => {
            if layout.current_column.contains(pos) || layout.parent_column.contains(pos) {
                let count = app.current_children_count();
                app.columns.move_selection(-1, count);
                app.invalidate_preview_cache();
            }
        }

        // -- Horizontal scroll (trackpad swipe) --
        MouseEventKind::ScrollLeft => {
            app.columns.back();
            app.invalidate_preview_cache();
        }
        MouseEventKind::ScrollRight => {
            if let Some(entry) = app.selected_entry() {
                if entry.is_dir {
                    let path = entry.path.clone();
                    app.columns.enter(path);
                    app.invalidate_preview_cache();
                }
            }
        }

        // -- Left click --
        MouseEventKind::Down(MouseButton::Left) => {
            let is_dbl = is_double_click(app, mouse.column, mouse.row);
            app.last_click = Some((mouse.column, mouse.row, std::time::Instant::now()));

            if layout.current_column.contains(pos) {
                // Click in current column: select the clicked row
                let scroll_offset = app.columns.current().scroll_offset;
                if let Some(row) = click_to_row(layout.current_column, mouse.row, scroll_offset) {
                    let count = app.current_children_count();
                    if row < count {
                        app.columns.current_mut().selected_index = row;
                        app.invalidate_preview_cache();

                        if is_dbl {
                            // Double-click: enter dir or toggle mark on file
                            if let Some(entry) = app.selected_entry() {
                                if entry.is_dir {
                                    let path = entry.path.clone();
                                    app.columns.enter(path);
                                    app.invalidate_preview_cache();
                                } else {
                                    let path = entry.path.clone();
                                    app.marks.toggle(&path);
                                }
                            }
                        }
                    }
                }
            } else if layout.parent_column.contains(pos) {
                // Click in parent column: go back to parent
                app.columns.back();
                app.invalidate_preview_cache();
            } else if layout.preview.contains(pos) {
                // Click in preview: enter selected directory
                if let Some(entry) = app.selected_entry() {
                    if entry.is_dir {
                        let path = entry.path.clone();
                        app.columns.enter(path);
                        app.invalidate_preview_cache();
                    }
                }
            }
        }

        // -- Right click: go back --
        MouseEventKind::Down(MouseButton::Right) => {
            app.columns.back();
            app.invalidate_preview_cache();
        }

        // -- Middle click: toggle mark --
        MouseEventKind::Down(MouseButton::Middle) => {
            if layout.current_column.contains(pos) {
                let scroll_offset = app.columns.current().scroll_offset;
                if let Some(row) = click_to_row(layout.current_column, mouse.row, scroll_offset) {
                    let count = app.current_children_count();
                    if row < count {
                        // Select the row first, then toggle mark
                        app.columns.current_mut().selected_index = row;
                        if let Some(path) = app.selected_path() {
                            app.marks.toggle(&path);
                        }
                        app.invalidate_preview_cache();
                    }
                }
            }
        }

        // Ignore Up, Drag, Moved
        _ => {}
    }
}
```

### Bug Fix: ensure_visible After Mouse Selection

The `ensure_visible` method exists on `App` but is never called (`#[allow(dead_code)]`). After clicking a row outside the current viewport, `selected_index` would change but `scroll_offset` wouldn't follow, making the selection invisible.

Call `ensure_visible` after any `selected_index` change. This requires knowing the list area height. Since we have the layout:

```rust
let list_height = layout.current_column.height.saturating_sub(1);  // -1 for header
app.ensure_visible(list_height);
```

Add `ensure_visible` calls after every selection change in the mouse handler. Also add it to the keyboard handler in `handle_main_analytics` after j/k/g/G movements — this was likely already a bug for large lists.

### macOS Gotcha: Ctrl+Click

Crossterm documentation states: "macOS reports Ctrl + left mouse button click as a right mouse button click." This means on macOS, Ctrl+Click = Right Click. Our mapping (right-click = go back) works naturally with this. No special handling needed.

### State Changes Summary

**File**: `crates/purifier-tui/src/app.rs`

```rust
// New fields on App:
pub terminal_size: Rect,                                    // Updated each frame
pub last_click: Option<(u16, u16, std::time::Instant)>,     // Double-click detection
```

Initialize in `App::new`:
```rust
terminal_size: Rect::default(),
last_click: None,
```

Reset `last_click` to `None` on navigation changes (h/l/Enter) to prevent false double-clicks across different directory contexts.

**File**: `crates/purifier-tui/src/main.rs`

Before draw:
```rust
app.terminal_size = terminal.size()?;
```

**File**: `crates/purifier-tui/src/input.rs`

- Complete rewrite of `handle_mouse` (see above)
- Add `use crate::ui::miller_layout;` and `use ratatui::layout::Position;`
- Add `click_to_row` and `is_double_click` helper functions
- Add `ensure_visible` calls after j/k in `handle_main_analytics`

**File**: `crates/purifier-tui/src/ui/mod.rs`

- Make `miller_layout` public (it's already `pub fn`)

### Tests

```rust
#[cfg(test)]
mod mouse_tests {
    use super::*;

    #[test]
    fn click_to_row_maps_correctly() {
        let pane = Rect::new(0, 1, 40, 20);  // y=1, height=20
        // Header at y=1, first data row at y=2
        assert_eq!(click_to_row(pane, 2, 0), Some(0));
        assert_eq!(click_to_row(pane, 3, 0), Some(1));
        assert_eq!(click_to_row(pane, 2, 5), Some(5));  // scroll_offset=5
        assert_eq!(click_to_row(pane, 1, 0), None);     // header
        assert_eq!(click_to_row(pane, 0, 0), None);     // above pane
    }

    #[test]
    fn click_to_row_returns_none_for_out_of_bounds() {
        let pane = Rect::new(0, 1, 40, 5);
        assert_eq!(click_to_row(pane, 6, 0), None);  // below pane (y=1+5=6)
    }

    #[test]
    fn double_click_detection_respects_timeout() {
        let mut app = app_with_entries();
        app.last_click = Some((10, 5, std::time::Instant::now()));
        assert!(is_double_click(&app, 10, 5));  // Same position, immediate

        app.last_click = Some((10, 5, std::time::Instant::now() - std::time::Duration::from_millis(500)));
        assert!(!is_double_click(&app, 10, 5));  // Same position, too slow

        app.last_click = Some((10, 5, std::time::Instant::now()));
        assert!(!is_double_click(&app, 11, 5));  // Different position
    }

    #[test]
    fn scroll_should_invalidate_preview_cache() {
        let mut app = app_with_entries();
        app.terminal_size = Rect::new(0, 0, 160, 40);
        // After implementing, verify preview cache is invalidated
        // (the current code does NOT do this — this test would catch the bug)
    }

    #[test]
    fn right_click_should_go_back() {
        let mut app = app_with_entries();
        app.terminal_size = Rect::new(0, 0, 160, 40);
        // Enter a directory first
        handle_key(&mut app, key(KeyCode::Char('l')));
        assert_eq!(app.columns.depth(), 2);

        // Right click anywhere should go back
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 50, row: 10,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse(&mut app, mouse);
        assert_eq!(app.columns.depth(), 1);
    }
}
```

---

## Implementation Order

1. **Fix 1** first — it's the most impactful visual improvement and changes the row building in both column renderers.
2. **Fix 4** second — the rules are pure data, zero risk of breaking the UI, and immediately improve classification coverage.
3. **Fix 5** third — mouse support builds on the existing layout infrastructure and fixes the scroll invalidation bug.
4. **Fix 3** fourth — add the help overlay and improve the status bar hints. Low-risk and adds discoverability.
5. **Fix 2** last — the scroll animation requires a tick mechanism and has the most moving parts.

## Files to Change

| File | Fix | Change |
|------|-----|--------|
| `columns_view.rs` | 1, 2 | Padded rows, fixed-width sizes, scroll for selected, ellipsis for others, sort indicator hint |
| `app.rs` | 2, 3, 5 | `name_scroll_tick/offset` fields, `PreviewMode::Help`, `advance_name_scroll()`, `terminal_size`, `last_click` |
| `input.rs` | 3, 5 | `?` key handler, Help mode input, full mouse handler rewrite, `ensure_visible` calls |
| `main.rs` | 2, 5 | Call `advance_name_scroll()` before draw, update `app.terminal_size` |
| `status_bar.rs` | 3 | Expanded help text in status bar |
| `ui/mod.rs` | 3 | Wire help overlay rendering, add `help_overlay` module |
| `ui/help_overlay.rs` | 3 | New file: help overlay centered popup |
| `ui/preview_pane.rs` | 5 | Gesture mapping reference section in Settings screen after Scan Profile |
| `rules/default.toml` | 4 | Expand from 32 to ~90 rules |
| `llm.rs` | 4 | Add system message to classification prompt |

## Verification

```bash
cargo test -p purifier-core --all-targets
cargo test -p purifier-tui --all-targets
cargo clippy -p purifier-core -p purifier-tui --all-targets -- -D warnings
cargo run -- ~/Downloads   # Check alignment, scroll animation, help overlay, mouse
cargo run -- ~             # Check new rules classify correctly
```

### Visual Checks

- [ ] Size column (e.g. "12.3 MB") is vertically aligned across all rows
- [ ] Safety badges (⚠/✓/✗) are vertically aligned across all rows
- [ ] Long filenames in non-selected rows show trailing `…`
- [ ] Selected row with long filename scrolls after 1 second pause
- [ ] `?` key opens a centered help overlay listing all keybindings
- [ ] Esc or `?` closes the help overlay
- [ ] Status bar shows readable keybinding hints
- [ ] Sort indicator row shows hint about `s` to cycle
- [ ] `node_modules`, `__pycache__`, `.next`, etc. show as Safe/BuildArtifact
- [ ] `~/.ssh`, `.env` files show as Unsafe/System
- [ ] `~/Movies`, `~/Pictures` show as Caution/Media
- [ ] `~/Library/Caches/Homebrew` shows as Safe/Cache
- [ ] `~/Library/Developer/Xcode/DerivedData` shows as Safe/BuildArtifact
- [ ] Left click on a row in the current column selects it
- [ ] Double-click on a directory enters it
- [ ] Double-click on a file toggles its mark
- [ ] Right click anywhere goes back to parent
- [ ] Click on parent column pane goes back to parent
- [ ] Click on preview pane enters the selected directory
- [ ] Middle click toggles mark on the clicked row
- [ ] Scroll wheel moves selection up/down with preview update
- [ ] Horizontal trackpad swipe navigates back/forward (ScrollLeft/ScrollRight)
- [ ] Selection stays visible after clicking near viewport edges (ensure_visible)
- [ ] Settings screen (`,` key) shows "Mouse & Trackpad" gesture reference after Scan Profile
- [ ] Gesture reference is NOT shown during Onboarding flow
