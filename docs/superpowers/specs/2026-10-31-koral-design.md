# Koral: a terminal built around sharky

## Context

sharky is a personal shell, rewritten from Ruby to Rust. Today it runs inside
kitty (`shell /home/mafu/.local/bin/sharky`, IosevkaTerm Nerd Font, 0.80 opacity,
no decorations, bound to `SUPER + Return` on Hyprland).

Koral is its companion terminal, written in Rust. Three reasons, in priority
order:

1. Identity: its own look, sharing the sharky theme. Native tabs in the spirit
   of Windows Terminal, but better looking.
2. Integration: the terminal understands sharky commands as units.
3. Later, not in v1: shared human/agent sessions.

Success means Koral replaces kitty on `SUPER + Return` for daily use.

## Goals (v1)

- Linux on Wayland/Hyprland.
- Native tab strip drawn by Koral, inside the window, with no tmux involved.
- Command stream with folds: each command's output can collapse; the running
  command stays pinned at the top while its output scrolls.
- Per-command status, duration, jump between prompts, copy one command's output.
- Colours from `theme.sharky`; no second theme format.
- Correct behaviour for full-screen apps: nvim, tmux, btop.
- Images and video through the kitty graphics protocol (see Images).
- Parity with what daily kitty use relies on (see Terminal parity).

All of these shipped in 1.0.0. Open at that release: the `SUPER + Return` switch,
which stays the user's decision, and the checks left to a person in
`docs/testing.md`.

## Non-goals (v1)

- Internal splits: Hyprland and tmux already cover them.
- Agent sessions, X11, macOS, Windows.
- Kittens, remote control and unicode image placeholders.
- Warp-style input cards: sharky's line editor stays in charge of input.
- Tear-out tabs and tab profiles.

## Anti-goals

- Koral must never be required by sharky. Marks are standard escapes, and other
  terminals keep working.
- Program output must not be able to hide content the user did not choose to
  hide (see Security).

## Approaches

Chosen by the user: `alacritty_terminal` for emulation, so effort goes to folds,
tabs, theme and integration.

Rejected: `libghostty-vt` (Zig in the build, young 0.x API) and a custom emulator
on `vte` (months of conformance work before the first fold).

## Architecture

New repository at `/home/mafu/hdd/Projetos/koral`, one crate, GPL-3.0-only to
match sharky. Local `git init` only; creating a GitHub repository needs the
user's approval. Library plus binary, so the core is tested with no window.

```
PTY reader thread (one per tab)
  bytes -> marks scanner -> vte Processor -> Term (grid)
                     \-> BlockStore (command, cwd, status, duration, range)
winit thread: input, tab strip, viewport (Term + BlockStore -> rows), wgpu
```

Facts already verified:

- `vte` 0.15 `osc_dispatch` has no case for OSC 133 or OSC 7; both go to
  `unhandled`. Koral scans for them in the byte stream before `Term` sees them.
- `alacritty_terminal` exposes `tty`, `Term::grid`, `grid_mut`, `resize` and
  `sync`. Do not use its `event_loop`: Koral owns the read loop so it can split
  chunks at marks and read the cursor at that exact point.
- `Term::renderable_content` follows unfolded display order, so folded views
  build rows from `grid()` directly. Selection and scrolling go through the same
  row mapping.

Dependencies: `alacritty_terminal`, `winit` 0.30 (0.31 is still beta), `wgpu`,
`glyphon` + `cosmic-text`, one Wayland clipboard crate. Pin a `glyphon` release
that matches the `wgpu` version. No async runtime, as in sharky.

Modules: `marks` (scanner), `blocks` (block store and state machine), `view`
(fold-aware row mapping), `tabs`, `render`, `theme`, `input`.

### Block anchors

Alacritty has no stable line identity: scrollback truncation shifts indexes and
resize reflows lines. Each block is anchored by a private-use zero-width character
on the first cell of its prompt line, so it travels with the cell.

The milestone 1 spike confirmed that anchors survive scrolling, narrowing and
widening reflow, truncation, `ED 2` and the alternate screen. `ED 3` removes them
with the scrollback. Writing to a cell drops its anchor, so it is placed on `B`,
after the prompt text, not on `A`, and sharky's redraw places it again.

### Native tabs

- Strip at the top of the window, in the same translucent background. The active
  tab is a solid shape that merges into the terminal area.
- Title: running command, otherwise the cwd basename, both from the marks.
- State per tab from `BlockStore`: running, last command ok, last command failed.
  This is what plain terminals cannot show.
- `+` button, close on hover and middle-click, drag to reorder, double-click on
  empty strip space does nothing special, and dragging empty strip space moves
  floating windows.
- Keys: `ctrl+shift+t`, `ctrl+shift+w`, `ctrl+tab`, `ctrl+shift+tab`, `alt+1..9`.
- One `Term` + PTY + `BlockStore` per tab. Background tabs keep reading.
- Closing a tab with a running command asks for confirmation.
- A single tab still shows the strip by default, with a setting to hide it.
- No minimize/maximize/close buttons: Hyprland owns the window.

### Theme and config

- `rust/src/config.rs` lives in a binary crate. Extract it to a zero-dependency
  path crate, `sharky-config`, inside the sharky repo; sharky uses it by path and
  Koral by git tag. No copied parser.
- `wave` keys are free-form, so no shell parser change is needed:
  `wave terminal.background @abyss`, `wave terminal.ansi1 @danger`. Sharky's own
  colour names come next, then a built-in palette.
- `$XDG_CONFIG_HOME/koral/config.sharky` uses sharky's config language, with its
  own file and its own `polyp` statements: font, size, opacity, shell, padding.
  Default shell: `sharky`, then `$SHELL`.
- `TERM=xterm-256color` with `TERM_PROGRAM=koral`.
  `shortcut: no koral terminfo entry, add one if an app needs it.`

### sharky changes

Owned by the agent that works in the sharky repo, not by this session. This
session writes them as a self-contained handoff at
`/home/mafu/hdd/Projetos/sharky-shell/rust/docs/koral-integration.md`, in English,
leaves it uncommitted, and changes nothing else in that repo. The handoff covers
the marks below and the `sharky-config` extraction, with the agreed wire format,
the tests expected, and the memory-capped cargo commands from `rust/PORTING.md`.

Koral does not wait for it: milestones 1 and 2 use byte fixtures in this format,
and milestone 3 uses the built-in palette until `sharky-config` has a git tag.

Marks live in `rust/src`, emitted only in interactive mode:

| Moment | Escape | Where |
| --- | --- | --- |
| before the prompt | `OSC 133;D;<status>` (after a command), `OSC 7`, `OSC 133;A` | `repl.rs`, before `read_line` |
| end of the prompt | `OSC 133;B` | end of the string passed to `read_line` |
| after parsing, before running | `OSC 133;C;cmdline_url=<percent-encoded>` | `shell.rs`, between `parse` and `run_program` |

Details the builder must handle:

- `Shell::eval` parses and runs in one call, and an incomplete parse keeps
  buffering. `C` must be emitted only after a complete parse, so continuation
  lines stay in the same block. A parse error goes straight to `D;2`.
- `Editor::render` re-emits the prompt on every key, so `B` repeats. Koral treats
  repeated marks as no-ops.
- `tests/interactive.rs` may assert exact output and need updating.

## Images

- `src/graphics.rs` parses kitty graphics commands (`a=t,T,p,q,d`; formats 24, 32
  and 100; `o=z`; chunks) and keeps the images. The marks scanner takes the APCs
  out of the stream, since `vte` ignores them.
- A placement rides on its top-left cell as a zero-width anchor in plane 16,
  above the block ids, so it scrolls, folds and clears with the text. A second
  anchor on the last row makes a clear take the image's whole height.
- Frames with no id on the same cell replace each other: mpv sends video this way.
- Images draw over cell backgrounds and under text, the cursor and the chrome.
- A cropped placement fills its cells: yazi draws an image as one tile per cell.
- Not supported: z-index, animation frames and unicode placeholders.

## Terminal parity

What daily kitty use relies on, shipped in steps after v0.1:

- Input: the kitty keyboard protocol, word, line and block selection, input
  methods and accents, font zoom.
- Rendering: every underline style and colour, cursor blink, box characters
  drawn without the font, chosen font weights and italic style.
- Desktop: program titles, bell marks and window urgency, notifications.
- Finding: scrollback search and clickable links, with OSC 8.

## Security

- Any program can print OSC 133, so a crafted file could fake blocks. Marks are
  ignored in the alternate screen, and only valid state transitions are accepted.
- Folds only start collapsed from an explicit user action or setting, so fake
  marks cannot silently hide output.
- Strip ESC and `\x1b[201~` from pasted text in bracketed paste mode.
- `cmdline_url` is display-only data, never executed.
- OSC 52 clipboard reads are disabled.
- Images arrive only inside the stream: file, temp-file and shared-memory media
  are refused, so a crafted file cannot make Koral read local paths.
- Image, store and transfer sizes are capped.
- Links open only on a deliberate click, with the real target shown first, and
  only for `http`, `https`, `mailto` and `file`.
- Titles and notifications from programs are cleaned and limited in size and rate.

## Milestones

Thin slices, TDD for all non-rendering logic, one conventional commit per logical
change. Follow sharky's memory-capped cargo commands from `rust/PORTING.md`.

0. **sharky handoff:** write `rust/docs/koral-integration.md` in the sharky repo.
   The user passes it to the sharky agent. Once that agent ships the marks, PTY
   tests assert the sequence and `ctrl+shift+z` prompt jumps work in kitty.
1. **Spike, throwaway:** headless test that mark anchors survive reflow, scrollback
   truncation and clear. If they fail, choose another anchoring and update this
   document.
2. **Headless core:** `marks`, `blocks`, `view`, tested by feeding bytes.
3. **Terminal baseline:** one window with font, transparency, theme, input,
   selection, clipboard, resize. Usable with nvim, tmux and btop.
4. **Native tabs:** strip, state indicators, keys, mouse, reorder, close
   confirmation.
5. **Folds:** toggles, pinned header, prompt jumps, copy output, status and duration.
6. **Daily use:** run alongside kitty, fix what hurts, then the user decides on
   switching `SUPER + Return`.

## Left to the builder

- Fold toggle keys, exact tab metrics, and config key names.
- Renderer detail: start with `glyphon` line caching; replace with a custom atlas
  only if the performance targets fail.

## Verification

- `cargo test`, `cargo clippy --all-targets`, `cargo build --release` in Koral.
- sharky, run by the sharky agent: PTY tests for the mark sequence, including
  continuation lines and parse errors; manual prompt jump in kitty.
- Handoff: reread `rust/docs/koral-integration.md` against this document, and check
  that `git status` in the sharky repo shows only that new file.
- Koral headless: resize, truncation, nested shells, fake marks, marks in the
  alternate screen, marks split across read chunks.
- Koral manual: `vttest`; nvim, tmux, btop and fastfetch; live resize with folds
  open; tabs under load, with a large `cat` in a background tab.
- Targets: cold start under 100 ms, typing latency that feels no worse than kitty,
  and no UI freeze while printing a 100 MB file. Measure against kitty.
- Accessibility: every tab and fold action reachable by keyboard; state indicators
  use shape as well as colour.

## Assumptions to check

- Tabs yes, splits no: Claude's call after the feature question went unanswered.
  Tabs later confirmed by the user. Images were requested later and are in.
- Separate repository, `sharky-config` extraction, and the `wave terminal.*` keys:
  Claude's calls.
- Multi-line paste into sharky has no bracketed paste yet, so pasted lines run one
  by one. Separate follow-up in sharky.
- The interactive config milestone in `PORTING.md` is unfinished; prompt colours
  may need it. Milestone 0 does not depend on it.
