# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Dates use
`YYYY-MM-DD`; an unreleased section carries the `Unreleased` marker instead of a date.

## [2.0.0-alpha.1] - 2026-11-25

First `2.0.0` alpha / nightly pre-release, adding block-centric CLI and UI workflows while keeping `1.2.1` as the STABLE release (`koral-bin` stays on `1.2.1`; `koral-bin-nightly` tracks `2.0.0-alpha.1`).

### Added

- Block output reuse CLI shorthands: `koral @-1`, `koral @-N`, `koral @err`, and `koral @<id>`, plus `--block` selection on `koral ctl get-block-output`.
- Native block diff engine (`src/diff.rs`), `koral diff-blocks` CLI command, and scrollable inline diff overlay (`Ctrl+Shift+Y`).
- Block-scoped search (`Ctrl+Shift+B` / `Alt+B`), right-edge clickable block status minimap (`block_minimap on|off`), and `Ctrl+Shift+L` jump to the latest failed command.
- Smart opt-in auto-folding for long successful commands (`auto_fold_lines N`, `0` = off; failures stay expanded).
- Expanded scrollable Settings UI (`Section::Blocks`, `tab_bar`, `cursor_shape`, `cursor_blink`, `line_height`, `bold_italic_weight`, `present_mode`, `auto_fold_lines`, `block_minimap`) and 10 additional built-in themes (25 total, including `github-dark`, `github-dark-dimmed`, and `github-light`).
- Separate pre-release AUR recipe `packaging/aur/koral-bin-nightly/PKGBUILD` (`koral-bin-nightly`) while keeping `koral-bin` `1.2.1` as STABLE.
- Live `bash`, `zsh`, and `fish` PTY integration smoke tests in `tests/shell.rs`.
- Linux AT-SPI accessibility bridge via `accesskit_winit` (publishes window, terminal,
  and visible viewport line nodes lazily when an AT-SPI consumer is active).
- Arch Linux packaging recipe (`packaging/arch/PKGBUILD`) and compiled `xterm-koral`
  terminfo bundling in `flake.nix` and `.github/workflows/release.yml`.
- `koral ctl get-block-output [--pane ID] [--block ID] [--json]`, streaming
  `koral ctl watch [--pane ID] [--json]` NDJSON lifecycle hooks, and Linux `SO_PEERCRED`
  same-UID socket authentication.
- Kitty graphics protocol animation support (`a=f` frames, `a=a` playback/loop control,
  `a=c` frame composition) scheduled on the event loop without busy polling.
- `OSC 5522` extended clipboard protocol (`text/plain` and `image/png`, `clipboard` and
  `primary` targets) gated by `clipboard_write` / `clipboard_read` policies.
- `CSI ?2027` (grapheme clustering mode + `DECRQM` query) keeping ZWJ emoji, skin-tone
  modifiers, variation selectors, and regional indicator flags in single cell clusters.
- Extended `koral ctl` with `--pane ID` target selectors (`send-text`, `set-title`,
  `close-pane`, `list-blocks`) and `list-blocks [--json]` (`{"version":1,"blocks":[...]}`).
- Named session layout save/restore (`save-session`, `restore-session`,
  `list-sessions [--json]`) stored atomically (`0600` JSON `version: 1`) under
  `$XDG_STATE_HOME/koral/sessions` without scrollback or command history.
- Built-in `high-contrast` accessibility theme (WCAG AAA 21:1 foreground/background contrast).
- `koral +ssh [--bootstrap-terminfo] [SSH_ARGS...]` OpenSSH wrapper that falls back to
  `TERM=xterm-256color` by default and compiles `xterm-koral` remotely only when requested.
- `OSC 9;4` tab progress indicators and `OSC 22` pointer cursor shapes (automatically reset when leaving the alternate screen).
- `ligatures on|off` and `font_features` (`+tag`, `-tag`, `tag=N` OpenType tags) in `koral.conf`
  and **Settings → Typography → Font ligatures**.
- Linux X11 support (`WM_CLASS` via `WindowAttributesExtX11` and X11 runtime library
  packaging in `flake.nix`).
- `koral ctl --json list` for versioned JSON tab/pane inspection (`{"version":1,"tabs":[...]}`)
  and JSON error reporting.
- `xterm-koral` terminfo source (`assets/xterm-koral.terminfo`) and explicit
  `koral +terminfo install` command, with automatic fallback to `xterm-256color` when not
  installed locally.
- Automatic shell integration (`OSC 133` command marks and `OSC 7` working directory)
  for `bash`, `zsh`, and `fish` without modifying user startup files (`shell_integration
  on|off`, on by default).
- **Appearance → System title bar** requests native window decorations, including the system
  close button. Off by default. Config key: `window_decorations` (`on` or `off`).
- Pull-request CI configuration, security and contribution guides, and issue and PR templates.
- Fuzz targets for marks, graphics, IPC decoding, and config parsing. The stateful graphics
  target passed an instrumented smoke run: 1,703,276 executions in 61 seconds without a crash.
- Regression coverage for mixed render-cache hits and misses, scanner boundaries, and PTY
  staging limits.

### Changed

- Decoupled PTY reading from terminal parsing and optimized ASCII row fills and keystroke redraw skipping to lower input-to-photon latency and bulk throughput overhead.
- Configuration moved to `~/.config/koral/koral.conf` (kitty-style `key value`,
  `keybind shortcut=action`, `#` comments), dropping the `sharky-config` dependency.
  Existing `config.sharky` / `config.koral` files are converted once to `koral.conf` and
  left untouched.
- `present_mode auto|fifo|mailbox|immediate` selects the presentation mode at startup.
  `auto` defaults to FIFO, and unsupported requests fall back to FIFO.
- PTY burst pacing adapts to the monitor refresh rate (with 60 Hz fallback) and local
  keyboard input triggers an immediate redraw.
- Startup defers the full font inventory scan to the background loader and uses a fast
  monospace fallback lookup first.
- Clipboard writes and notifications use independent policies. Consent is visible in Settings,
  uses Ctrl+Shift+Enter to allow and Escape to deny, and leaves ordinary Enter available to the PTY.
- The benchmark isolates clean profiles, pins the executable, and rejects latency samples
  without confirmation of the expected input from inside the PTY.
- Each terminal owns its row projection. Rendering, scrolling, search, and block actions
  reuse it, and closing the terminal drops its cache.

### Fixed

- Confirmed and allowed notifications share the same delivery spacing.
- The stream scanner preserves bytes and new sequence starts at chunk and size boundaries.
- Reaching the PTY staging limit no longer ends an ongoing output burst.

## [1.2.1] - 2026-11-21

Maintenance release on the stable `1.2.x` line.

### Changed

- Bumped `sharky-config` to `v4.0.0-alpha.2` and refreshed the lockfile and the
  `koral-bin` Nix package checksum.
- Refreshed the README with the current v1.2.1 showcase and benchmark table.

## [1.2.0] - 2026-11-21

### Added

- Native split panes, configurable keybindings, the `koral ctl` IPC interface, and a live
  Settings tab.
- Fourteen built-in themes with surface mixing.
- Launch flags (`--cwd`, `--title`, `--class`, `-e`), inotify config reload, a search match
  counter, local path links, and a scrollback indicator.
- Fallback glyph alignment for missing or proportional fonts.

### Fixed

- Preserve image anchors on line erase, flush synchronized updates, and add Unicode
  placeholder support for Kitty graphics.
- Discard a suboptimal surface texture before reconfiguring the swap chain.

### Changed

- Simplified a nested let-chain in `main.rs`.

## [1.1.2] - 2026-11-18

Maintenance release; see `git log v1.1.1..v1.1.2` for the exact changes.

## [1.1.1] - 2026-11-18

### Changed

- Loaded fallback font faces before the first frame needed them.
- Split text shaping from preparation in the frame log and evicted old shaped lines instead
  of clearing the cache.
- Logged slow frames with per-phase costs.
- Bound each frame's pipeline state in every picture band.

### Fixed

- Kept or restored graphics behavior around stacked z placements, positional deletes,
  alternate-screen transitions, and synchronized updates.
- Reported mouse motion so a grabbed program can drag.

## [1.1.0] - 2026-11-18

### Added

- Nix package for the prebuilt release binary (`koral-bin`, the default flake package).
- Desktop entry, icon, and Nix wrapper for runtime dependencies.

### Changed

- Substituted a missing configured font with a Nerd Font.
- Evicted image placements without a client image ID before on-screen ones.
- Answered unsupported graphics actions and transmission modes.
- Drew pictures at the size requested by the protocol.

## [1.0.3] - 2026-11-01

Maintenance release; see `git log v1.0.2..v1.0.3` for the exact changes.

## [1.0.2] - 2026-11-01

Maintenance release; see `git log v1.0.1..v1.0.2` for the exact changes.

## [1.0.1] - 2026-11-01

Maintenance release; see `git log v1.0.0..v1.0.1` for the exact changes.

## [1.0.0] - 2026-11-01

Initial tagged release. See `git log v0.1.0..v1.0.0` for the full history.
