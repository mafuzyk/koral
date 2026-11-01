# Koral Roadmap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task, inline. Do not spawn subagents unless the user asks. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish Koral and ship v0.1.0, then close the gap with kitty and reach v1.0.0 as the daily terminal for two people on Hyprland.

**Architecture:** Each phase ends in a tagged release, so the AUR package keeps working while parity lands. Terminal logic stays in the library and is tested with no window; `src/main.rs` only routes events. `alacritty_terminal` already provides selection kinds, regex search, OSC 8 cells and keyboard mode tracking, so Koral uses those instead of writing its own.

**Tech Stack:** Rust 2024, `alacritty_terminal` 0.26, `winit` 0.30, `wgpu` 30, `glyphon` 0.12, `sharky-config` (git tag `v3.0.0`, added in Task 4; the only new dependency).

**Spec:** `docs/superpowers/specs/2026-10-31-koral-design.md`, updated by Task 1.

## Global Constraints

- Licence: GPL-3.0-only, everywhere it is stated.
- Platform: Linux on Wayland. Both users run Hyprland. X11, macOS and Windows stay out.
- Out of this plan: agent sessions, splits, kittens, remote control, unicode image placeholders.
- Everything on disk is English. Conventional Commits, one logical change per commit, no `Co-authored-by`, no AI attribution.
- Creating the GitHub repository, every `git push`, every GitHub release and every AUR push needs the user's ok in chat at that moment.
- The sharky repository is read-only for this plan.
- No production code before a test was seen failing for the right reason.
- Never run `matugen` against the real config. GUI checks run on the virtual monitor (nested headless `sway`, `grim`, `wtype`). Do not capture the user's screen without consent.

**Gate**, used by every task and release:

```bash
systemd-run --user --scope --quiet -p MemoryMax=3G -- cargo test -j 4
systemd-run --user --scope --quiet -p MemoryMax=3G -- cargo clippy -j 4 --all-targets
systemd-run --user --scope --quiet -p MemoryMax=3G -- cargo build --release -j 4
```

Expected: 0 failed, and clippy shows only the existing `draw_terminal` too-many-arguments warning.

## Review Focus

1. A machine with no sharky and no `theme.sharky`: Koral starts on `$SHELL` with the built-in palette, tab titles come from programs, and search and links work. Pinned in Tasks 5 and 14.
2. Upgrading with an existing `config.koral`, or with both files present: no setting is lost and an existing `config.sharky` is never overwritten. Pinned in Task 4.
3. A search match inside a folded command: the block opens instead of the match being invisible. Pinned in Task 17.
4. Hostile output: a link whose text lies about its target, a `javascript:` URI, control characters in a title, a notification flood. Each is refused, shown or limited. Pinned in Tasks 14, 16 and 18.
5. ABNT2 typing: `ç`, `ã` by dead key, and `ctrl+ç` keep working in legacy mode, and Koral's own shortcuts keep working while an app has the kitty keyboard protocol on. Pinned in Tasks 6 and 8.

---

## Phase 1: v0.1.0, what exists today

### Task 1: Make the repository presentable

**Files:**
- Modify: `src/main.rs`, `.gitignore`, `docs/superpowers/specs/2026-10-31-koral-design.md`
- Create: `README.md`, `docs/RELEASING.md`, `docs/screenshot.png`, `tests/version.rs`

**Interfaces:**
- Produces: `koral --version` prints `koral <Cargo version>` and exits 0 without opening a window. `docs/RELEASING.md` is the release procedure every later phase follows.

- [ ] **Step 1: Write the failing test** `tests/version.rs::version_flag_prints_the_crate_version`: run `env!("CARGO_BIN_EXE_koral")` with `--version`, assert success and stdout equal to `format!("koral {}\n", env!("CARGO_PKG_VERSION"))`.
- [ ] **Step 2: Run** `cargo test -j 4 --test version`. Expected: FAIL. The binary has no argument handling, so it fails on the window or the output, not on a compile error.
- [ ] **Step 3: Implement** the check at the top of `main()`, using `std::env::args`. No argument crate.
- [ ] **Step 4: Run the same test.** Expected: PASS.
- [ ] **Step 5: Add** `/packaging/*.pkg.tar.zst`, `/packaging/src/`, `/packaging/pkg/` and `/packaging/*.tar.gz` to `.gitignore`.
- [ ] **Step 6: Update the spec.** Search and clickable links leave Non-goals. Add a `## Terminal parity` section listing Tasks 6 to 18. The config file becomes `config.sharky`, as in Task 4. Replace the `wave tab.active` example with the keys of Task 5. Add to Security: links open only on a deliberate click, with the real target shown first; titles and notifications from programs are cleaned and limited.
- [ ] **Step 7: Take the screenshot** on the virtual monitor: two tabs, one folded command, an image.
- [ ] **Step 8: Write `README.md`:** one paragraph on what Koral is, the screenshot, install, the shortcuts table from `src/input.rs`, config, matugen, what works without sharky, licence GPL-3.0-only.
- [ ] **Step 9: Write `docs/RELEASING.md`,** following `rust/RELEASING.md` in sharky: bump `Cargo.toml` and `packaging/PKGBUILD`, run the Gate, tag, push, `gh release create`, `updpkgsums`, `makepkg -f`, `makepkg --printsrcinfo > .SRCINFO`, push to AUR.
- [ ] **Step 10: Run the Gate.**
- [ ] **Step 11: Commit** `feat: print the version` for the code, and `docs: add the readme, release guide and roadmap` for the rest, including this plan.

### Task 2: Create the public repository

**Files:** none.

**Interfaces:**
- Produces: `https://github.com/mafuzyk/koral`, default branch `main`.

- [ ] **Step 1: Scan the history.** Run `git log -p | grep -inE '(api[_-]?key|secret|password|token)[[:space:]]*[=:]'` and read every hit. Expected: none that is a credential; the graphics and config code may match on the word only.
- [ ] **Step 2: List the tracked files** with `git ls-files` and confirm nothing private or large is in them.
- [ ] **Step 3: Rename the branch** with `git branch -M main`, matching sharky.
- [ ] **Step 4: Ask the user for the ok** to create the public repository. Do not continue without it.
- [ ] **Step 5: Create and push:**

```bash
gh repo create mafuzyk/koral --public --source=. --remote=origin --description "A Rust terminal with native sharky shell support" --push
```

- [ ] **Step 6: Verify** with `gh repo view mafuzyk/koral --json visibility,defaultBranchRef,licenseInfo`. Expected: `PUBLIC`, `main`, GPL-3.0.

### Task 3: Ship v0.1.0 to the AUR

**Files:**
- Create: `packaging/PKGBUILD`, `packaging/.SRCINFO`

**Interfaces:**
- Consumes: `docs/RELEASING.md` from Task 1.
- Produces: AUR package `koral-bin`, installing the tarball that `.github/workflows/release.yml` builds for the tag. The user asked for a binary package, and local builds use `target-cpu=native`.

- [ ] **Step 1: Confirm the optional package names** with `pacman -Si` and the AUR (`sharky-shell-git`, matugen, later `libnotify`, `xdg-utils`).
- [ ] **Step 2: Write `packaging/PKGBUILD`:**

```bash
# Maintainer: mafuzyk <mafuzyk@gmail.com>

pkgname=koral
pkgver=0.1.0
pkgrel=1
pkgdesc="A Rust terminal with native sharky shell support"
arch=('x86_64')
url="https://github.com/mafuzyk/koral"
license=('GPL-3.0-only')
depends=('gcc-libs' 'glibc' 'hicolor-icon-theme' 'libxkbcommon' 'ttf-iosevkaterm-nerd' 'vulkan-icd-loader' 'wayland')
optdepends=('sharky-shell-git: folds, prompt jumps and command status')
makedepends=('cargo')
source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('SKIP')

prepare() {
  cd "$pkgname-$pkgver"
  export CARGO_HOME="$srcdir/cargo"
  cargo fetch --locked
}

build() {
  cd "$pkgname-$pkgver"
  export CARGO_HOME="$srcdir/cargo"
  export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$srcdir=."
  cargo build --release --locked --jobs 4
}

check() {
  cd "$pkgname-$pkgver"
  target/release/koral --version
}

package() {
  cd "$pkgname-$pkgver"
  install -Dm755 target/release/koral "$pkgdir/usr/bin/koral"
  install -Dm644 assets/koral.desktop "$pkgdir/usr/share/applications/koral.desktop"
  install -Dm644 assets/koral.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/koral.svg"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
```

- [ ] **Step 3: Ask the user for the ok,** then tag and publish:

```bash
git tag -a v0.1.0 -m "koral 0.1.0"
git push origin main v0.1.0
gh release create v0.1.0 --generate-notes
```

- [ ] **Step 4: Refresh the checksum** with `updpkgsums` in `packaging/`, which replaces `SKIP`.
- [ ] **Step 5: Build the package** with `makepkg -f` and run `namcap` on it if installed. Expected: `check()` prints `koral 0.1.0`.
- [ ] **Step 6: Inspect it** with `pacman -Qlp koral-0.1.0-1-x86_64.pkg.tar.zst`. Expected: the binary, desktop entry, icon and licence.
- [ ] **Step 7: Write** `.SRCINFO` with `makepkg --printsrcinfo > .SRCINFO`, and commit `build: package koral for the AUR`.
- [ ] **Step 8: Ask the user for the ok,** then push `packaging/PKGBUILD` and `.SRCINFO` to `ssh://aur@aur.archlinux.org/koral.git` with message `upgpkg: koral 0.1.0`, and push the commit to GitHub.
- [ ] **Step 9: Ask the user and their girlfriend to install it** with `yay -S koral-bin`. Their result closes the phase, since it is the first run outside this machine.

---

## Phase 2: v0.2.0, the sharky language and theme

### Task 4: Use sharky's config language in `config.sharky`

Koral and sharky share the language, not the file: Koral reads `~/.config/koral/config.sharky`, sharky reads `~/.config/sharky/config.sharky`. Koral keeps its `polyp` instruction, so only the tokenizer is shared; `sharky_config::parse` would refuse `polyp`.

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`, `src/config.rs`, `src/main.rs`, `tests/config.rs`

**Interfaces:**
- Consumes: `sharky_config::tokenize(line: &str) -> Result<Vec<String>, String>`.
- Produces: `config::path()` returning `koral/config.sharky`; `config::migrate(dir: &Path) -> std::io::Result<()>`; private `comment_start(line: &str) -> Option<usize>`.

- [ ] **Step 1: Write the failing tests** in `tests/config.rs`:

```rust
// migrate
assert_eq!(read(dir, "config.sharky"), "polyp font_size 14\n"); // after migrate with only config.koral
assert!(!dir.join("config.koral").exists());
// migrate with both files: config.sharky keeps its text, config.koral stays untouched
// migrate with neither file: Ok(()) and nothing created
assert!(config::path().unwrap().ends_with("koral/config.sharky"));
// set keeps comments, and is not fooled by // inside quotes
assert_eq!(set("polyp font a // why\n", "font", "c"), "polyp font c // why\n");
assert_eq!(set("polyp font \"a // b\"\n", "font", "c"), "polyp font c\n");
// sharky ends a bare word at //, unlike Koral's old tokenizer
assert_eq!(set("polyp font a//why\n", "font", "c"), "polyp font c //why\n");
```

Each migrate test uses its own temporary directory under `std::env::temp_dir()`.

- [ ] **Step 2: Run** `cargo test -j 4 --test config`. Expected: FAIL on the missing `migrate` and on the path.
- [ ] **Step 3: Add the dependency:** `sharky-config = { git = "https://github.com/mafuzyk/sharky-shell", tag = "v3.0.0" }`.
- [ ] **Step 4: Replace the local tokenizer** with `sharky_config::tokenize` and remove its `shortcut:` comment.
- [ ] **Step 5: Implement `comment_start`:** the first `//` offset `i` where `tokenize(&line[..i])` equals the tokens of the whole line. A `//` inside quotes leaves the prefix unterminated, so it does not count. Update any existing test that asserts Koral's old tokenizer messages or its `a//b` word.
- [ ] **Step 6: Implement `migrate`** as a `std::fs::rename` that runs only when `config.sharky` is absent and `config.koral` exists. Change the temporary name in `save` to use `sharky.tmp`.
- [ ] **Step 7: Call `migrate`** in `App::new` before reading the config, printing errors to stderr. Change the message to `koral: config.sharky {error}`.
- [ ] **Step 8: Run the Gate.** Expected: all pass, including the existing `set`, `update` and quoting tests.
- [ ] **Step 9: Commit** `feat: read config.sharky with sharky's tokenizer`.

### Task 5: Colours from `theme.sharky`

**Files:**
- Modify: `src/theme.rs`, `src/matugen.rs`, `tests/theme.rs`

**Interfaces:**
- Consumes: `sharky_config::parse_theme(source, file) -> Result<sharky_config::Config, ConfigError>`, with `colors` for `coral` and `styles` for `wave`. Do not use `sharky_config::load`: it also validates the user's shell config.
- Produces: `Theme::with_sharky(source: &str, file: &str) -> Result<Theme, String>`; `Theme::sharky() -> Theme`, reading `<config home>/sharky/theme.sharky`.

Precedence: matugen when enabled and its colours exist, then `theme.sharky`, then the built-in palette.

Keys, in order of priority:
1. `wave terminal.<slot> @name` or `#RRGGBB`, with the slot names `Theme::with_colors` uses: `background`, `foreground`, `cursor`, `accent`, `success`, `failure`, `muted`, `ansi0` to `ansi15`.
2. Sharky's own colour names, so an existing theme with no `terminal.*` line still changes Koral: `abyss` for background, `foam` for foreground, `sea` for accent and cursor, `kelp` for success, `danger` for failure.
3. The built-in value.

- [ ] **Step 1: Write the failing tests** in `tests/theme.rs`:
  - `sharky_names_colour_a_theme_with_no_terminal_keys`: `coral abyss #010203` gives `background == hex("#010203")`.
  - `terminal_keys_beat_sharky_names`: `coral abyss #010203`, `coral ink #0A0B0C`, `wave terminal.background @ink` gives `#0A0B0C`.
  - `terminal_keys_take_a_literal_colour`: `wave terminal.ansi1 #112233`.
  - `an_unknown_reference_leaves_the_builtin_value`: `wave terminal.cursor @nothing` equals `Theme::default().cursor`.
  - `a_damaged_theme_is_an_error`: `coral abyss nope` returns `Err`.
- [ ] **Step 2: Run** `cargo test -j 4 --test theme`. Expected: FAIL on the missing `with_sharky`.
- [ ] **Step 3: Extract** the name-to-slot match from `with_colors` into `fn slot(&mut self, name: &str) -> Option<&mut Rgb>`, and implement `with_sharky` on it.
- [ ] **Step 4: Implement `Theme::sharky`:** missing file or `Err` gives `Theme::default()`, and a damaged file prints to stderr. Theme errors must not stop the terminal.
- [ ] **Step 5: Change `matugen::theme`** to fall back to `Theme::sharky`. Make `modified()` return the times of both files so `watch` reloads on either.
- [ ] **Step 6: Check on the virtual monitor** with a temporary `XDG_CONFIG_HOME`: a theme file changes the colours, an edit reloads within 2 s, and no sharky folder gives the built-in palette.
- [ ] **Step 7: Run the Gate, then commit** `feat: take colours from theme.sharky`.
- [ ] **Step 8: Release 0.2.0** by `docs/RELEASING.md`, with the user's ok for each push.

---

## Phase 3: v0.3.0, input

### Task 6: Kitty keyboard protocol

**Files:**
- Create: `src/keyboard.rs`, `tests/keyboard.rs`
- Modify: `src/lib.rs`, `src/pty.rs`, `src/main.rs`

**Interfaces:**
- Produces:

```rust
pub enum Stage { Press, Repeat, Release }
pub struct KeyPress<'a> {
    pub key: &'a Key,
    pub unshifted: &'a Key, // from KeyEventExtModifierSupplement::key_without_modifiers
    pub text: Option<&'a str>,
    pub mods: ModifiersState,
    pub stage: Stage,
}
/// None means "not a kitty sequence": the caller falls back to `input::encode`.
pub fn encode(press: &KeyPress, mode: TermMode) -> Option<Vec<u8>>
```

- [ ] **Step 1: Write the failing tests** in `tests/keyboard.rs`. `D` is `TermMode::DISAMBIGUATE_ESC_CODES`, `A` is `REPORT_ALL_KEYS_AS_ESC`, `E` is `REPORT_EVENT_TYPES`, `L` is `REPORT_ALTERNATE_KEYS`, `T` is `REPORT_ASSOCIATED_TEXT`.

| Test | Mode | Input | Bytes |
| --- | --- | --- | --- |
| `escape_is_disambiguated` | D | Esc | `\x1b[27u` |
| `ctrl_letter_is_disambiguated` | D | ctrl+a | `\x1b[97;5u` |
| `alt_letter_is_disambiguated` | D | alt+a | `\x1b[97;3u` |
| `plain_text_stays_text` | D | `a` | None |
| `accents_stay_text` | D | `ç` | None |
| `plain_enter_stays_legacy` | D | Enter | None |
| `shift_enter_is_reported` | D | shift+Enter | `\x1b[13;2u` |
| `arrows_keep_their_form` | D | ctrl+Up | `\x1b[1;5A` |
| `all_keys_report_letters` | A | `a` | `\x1b[97u` |
| `all_keys_report_modifier_keys` | A | Shift | `\x1b[57441;2u` |
| `associated_text_is_added` | A+T | `a` | `\x1b[97;;97u` |
| `alternate_key_is_added` | A+L | shift+a | `\x1b[97:65;2u` |
| `release_is_reported` | A+E | release of `a` | `\x1b[97;1:3u` |
| `repeat_is_reported` | A+E | repeat of `a` | `\x1b[97;1:2u` |
| `enter_release_needs_all_keys` | D+E | release of Enter | None |
| `legacy_mode_sends_nothing_new` | empty | ctrl+a | None |

  Add to `tests/input.rs`: `ctrl_cedilla_stays_silent_in_legacy_mode` (`encode` with ctrl+`ç` returns None, today's behaviour) and to `tests/session.rs` `an_app_can_turn_the_keyboard_protocol_on`: feed `\x1b[>1u`, assert the mode contains `DISAMBIGUATE_ESC_CODES`.

- [ ] **Step 2: Run** `cargo test -j 4 --test keyboard --test session`. Expected: FAIL on the missing module and the mode bit.
- [ ] **Step 3: Set** `kitty_keyboard: true` in the `Config` built by `Terminal::spawn`. Without it `alacritty_terminal` ignores the protocol.
- [ ] **Step 4: Implement `keyboard::encode`.** Leave a `shortcut:` comment that keypad keys and media keys report as their legacy form, and that modifier keys always report their left code, to upgrade if an app needs them.
- [ ] **Step 5: Route it in `window_event`.** Koral's own `binding` keeps priority on press and repeat. Handle `ElementState::Released` by calling `keyboard::encode` only when the mode has `REPORT_EVENT_TYPES`. Call `input::encode` when `keyboard::encode` returns None.
- [ ] **Step 6: Run the Gate.**
- [ ] **Step 7: Check on the virtual monitor** with `kitten show-key -m kitty`: `ctrl+a` and `shift+Enter` print their CSI u forms, and `ctrl+shift+t` still opens a tab.
- [ ] **Step 8: Commit** `feat: speak the kitty keyboard protocol`.

### Task 7: Word, line and block selection

**Files:**
- Modify: `src/input.rs`, `src/main.rs`, `tests/input.rs`, `tests/session.rs`

**Interfaces:**
- Produces: `input::Clicks { .. }` with `press(&mut self, cell: (i32, usize), now: Instant) -> u8` returning 1, 2 or 3; `input::selection_kind(clicks: u8, mods: ModifiersState) -> SelectionType`.

- [ ] **Step 1: Write the failing tests:**
  - `clicks_on_one_cell_count_up`: three presses 100 ms apart give 1, 2, 3, and a fourth gives 1.
  - `a_slow_click_starts_again`: presses 500 ms apart give 1, 1. The window is 400 ms.
  - `a_click_elsewhere_starts_again`: a different cell gives 1.
  - `selection_kind`: 1 is `Simple`, 2 is `Semantic`, 3 is `Lines`, any count with ctrl+alt is `Block`.
  - `tests/session.rs`: on a grid holding `foo bar`, `Selection::new(SelectionType::Semantic, <point in bar>, Side::Left)` gives `selection_to_string()` of `bar`, proving Koral's anchors do not split words.
- [ ] **Step 2: Run** `cargo test -j 4 --test input --test session`. Expected: FAIL on the missing `Clicks`.
- [ ] **Step 3: Implement** them and add `clicks: Clicks` to `App`.
- [ ] **Step 4: Use them in `press_terminal`,** replacing the fixed `SelectionType::Simple`. Double and triple clicks copy to the primary selection on release, as dragging does.
- [ ] **Step 5: Run the Gate, then commit** `feat: select words, lines and blocks`.

### Task 8: IME and accents

**Files:**
- Modify: `src/main.rs`, `src/frame.rs`, `tests/render.rs`

**Interfaces:**
- Produces: `frame::draw_preedit(frame: &mut Frame, text: &str, x: f32, y: f32, layout: Layout, theme: &Theme)`.

- [ ] **Step 1: Write the failing test** in `tests/render.rs`: `preedit_text_is_underlined_at_the_cursor`: after `draw_preedit` the frame has one `TextLine` at `(x, y)` holding the text and one underline quad as wide as the text in cells.
- [ ] **Step 2: Run** `cargo test -j 4 --test render`. Expected: FAIL on the missing function.
- [ ] **Step 3: Implement it.**
- [ ] **Step 4: Wire `main.rs`:** `window.set_ime_allowed(true)` in `resumed`; handle `WindowEvent::Ime`: `Commit(text)` follows the same path as typed text (settings field, search field, or `write_active`), `Preedit(text, _)` stores `App.preedit: Option<String>`, `Disabled` clears it. After each draw, call `set_ime_cursor_area` with the cursor cell.
- [ ] **Step 5: Run the Gate.**
- [ ] **Step 6: Check on the virtual monitor** with `wtype -k dead_tilde a`. Expected: `ã` reaches the shell. If `wtype` cannot compose, record it and rely on Step 7.
- [ ] **Step 7: Ask the user to check** on ABNT2: `ç`, `ã`, `é`, `ctrl+c`, `ctrl+shift+,`.
- [ ] **Step 8: Commit** `feat: accept text from input methods`.

### Task 9: Font zoom

**Files:**
- Modify: `src/input.rs`, `src/main.rs`, `tests/bindings.rs`, `tests/input.rs`

**Interfaces:**
- Produces: `Action::ZoomIn`, `Action::ZoomOut`, `Action::ZoomReset`; `input::zoomed(size: f32, delta: f32) -> f32`, clamped to 6.0..=72.0.

Keys: `ctrl+shift+=` or `+`, `ctrl+shift+-` or `_`, `ctrl+shift+0` or `)`. Step 1 point. Zoom applies to every tab and is not saved, as in kitty.

- [ ] **Step 1: Write the failing tests:** each key maps to its action in `tests/bindings.rs`, and `zoomed(71.5, 1.0) == 72.0`, `zoomed(6.5, -1.0) == 6.0`.
- [ ] **Step 2: Run** `cargo test -j 4 --test bindings --test input`. Expected: FAIL on the missing actions.
- [ ] **Step 3: Implement.** Add `zoom: f32` to `App`, call `renderer.set_font` then `resize_all`, as `change_setting` does.
- [ ] **Step 4: Run the Gate, then commit** `feat: zoom the font with the keyboard`.
- [ ] **Step 5: Release 0.3.0** by `docs/RELEASING.md`, with the user's ok for each push.

---

## Phase 4: v0.4.0, rendering

### Task 10: Underline styles and colours

**Files:**
- Modify: `src/frame.rs`, `tests/render.rs`

**Interfaces:**
- Produces: `frame::Underline { Single, Double, Curl, Dotted, Dashed }`; `frame::underline(style: Underline, x: f32, y: f32, width: f32, thickness: f32, color: [f32; 4]) -> Vec<Quad>`, where `y` is the top of the line.

- [ ] **Step 1: Write the failing tests:**
  - `single_is_one_quad`, `double_is_two_quads_apart` (gap of `2 * thickness`).
  - `dashed_and_dotted_leave_gaps`: the quads cover less than `width`.
  - `curl_covers_the_whole_width`: quads touch every pixel column of `width`, with `y` staying inside a band of `3 * thickness`.
  - `curl_joins_between_cells`: the last quad of a cell and the first of the next differ by at most `thickness` in `y`. Compute the phase from absolute `x`.
  - `an_underline_takes_its_own_colour`: draw a term fed `\x1b[4:3;58:2::255:0:0mx`, assert a quad with red.
- [ ] **Step 2: Run** `cargo test -j 4 --test render`. Expected: FAIL on the missing function.
- [ ] **Step 3: Implement,** map `Flags::UNDERLINE`, `DOUBLE_UNDERLINE`, `UNDERCURL`, `DOTTED_UNDERLINE` and `DASHED_UNDERLINE`, and use `cell.underline_color()` resolved through `theme.resolve`, falling back to `fg`.
- [ ] **Step 4: Run the Gate, then commit** `feat: draw every underline style`.

### Task 11: Cursor blink

**Files:**
- Create: `src/blink.rs`, `tests/blink.rs`
- Modify: `src/lib.rs`, `src/frame.rs`, `src/pty.rs`, `src/main.rs`

**Interfaces:**
- Produces: `Blink::new(now: Instant)`, `reset(&mut self, now: Instant)`, `visible(&self, now: Instant) -> bool`, `next(&self, now: Instant) -> Option<Instant>`. Half period 530 ms; stops, visible, 15 s after the last reset, as kitty does.
- `draw_terminal` takes `cursor: CursorView { focused: bool, visible: bool }` in place of `focused`, so it does not grow an argument.

- [ ] **Step 1: Write the failing tests:** `shows_then_hides_then_shows` (0, 530, 1060 ms), `stops_visible_after_fifteen_seconds` (`visible` true and `next` None at 15.1 s), `reset_restarts_the_clock`.
- [ ] **Step 2: Run** `cargo test -j 4 --test blink`. Expected: FAIL on the missing module.
- [ ] **Step 3: Implement.** Blink only when `term.cursor_style().blinking`, the window is focused and `scroll == 0`.
- [ ] **Step 4: Wire `main.rs`:** reset on key press and on `Event::CursorBlinkingChange` (add `UserEvent::Blink`). `about_to_wait` sets `ControlFlow::WaitUntil` from the earlier of `blink.next` and the close-confirm deadline, otherwise `ControlFlow::Wait`.
- [ ] **Step 5: Run the Gate.**
- [ ] **Step 6: Check on the virtual monitor:** `printf '\e[1 q'` blinks, `printf '\e[2 q'` stops, and CPU is idle after 15 s (`top -b -n1 -p <pid>`).
- [ ] **Step 7: Commit** `feat: blink the cursor when a program asks`.

### Task 12: Box and block characters

**Files:**
- Create: `src/boxes.rs`, `tests/boxes.rs`
- Modify: `src/lib.rs`, `src/frame.rs`

**Interfaces:**
- Produces: `boxes::quads(ch: char, x: f32, y: f32, width: f32, height: f32, thickness: f32, color: [f32; 4]) -> Option<Vec<Quad>>`. None means the font draws it.

Covered: straight lines U+2500 to U+2503, the corners, tees and crosses to U+254B, doubles U+2550 to U+256C, half lines U+2574 to U+257F, block elements U+2580 to U+2595, shades U+2591 to U+2593 at alpha 0.25, 0.5 and 0.75, quadrants U+2596 to U+259F. Heavy is `2 * thickness`.

Left to the font, with a `shortcut:` comment: arcs U+256D to U+2570, diagonals U+2571 to U+2573, dashed lines, and Powerline. Quads cannot draw curves; a mask atlas would be the upgrade.

- [ ] **Step 1: Write the failing tests** (cell 10×20, thickness 2):
  - `a_vertical_line_spans_the_cell`: `│` is one quad, height 20, centred.
  - `a_horizontal_line_spans_the_cell`: `─`, width 10.
  - `a_cross_reaches_all_four_edges`: `┼`.
  - `quads_land_on_whole_pixels`: at `x = 10.4` every quad edge is whole, so neighbours leave no seam.
  - `a_full_block_fills_the_cell`, `an_upper_half_fills_the_top`.
  - `a_light_shade_is_translucent`: `░` has alpha 0.25.
  - `arcs_stay_with_the_font`: `╭` is None, and so is `a`.
- [ ] **Step 2: Run** `cargo test -j 4 --test boxes`. Expected: FAIL on the missing module.
- [ ] **Step 3: Implement** with a table of per-arm weights for the line characters.
- [ ] **Step 4: Use it in `draw_terminal`:** when `quads` returns Some, push them and add a space to the text span, so the glyph is not drawn twice and columns stay put. Add to `tests/render.rs`: `a_drawn_box_leaves_no_glyph`.
- [ ] **Step 5: Run the Gate.**
- [ ] **Step 6: Compare on the virtual monitor:** `btop` and a box-drawing sample, Koral beside kitty, with no gaps between rows.
- [ ] **Step 7: Commit** `feat: draw box characters without the font`.

### Task 13: Font weights and styles

**Files:**
- Modify: `src/config.rs`, `src/render.rs`, `src/main.rs`, `tests/config.rs`, `tests/render.rs`

**Interfaces:**
- Produces: `render::Face { family: String, weight: u16, bold: u16, bold_italic: u16, oblique: bool }`, `Config::face(&self) -> Face`. `Renderer::new` and `set_font` take `&Face` in place of the font name. `render::attrs(face: &Face, bold: bool, italic: bool) -> Attrs<'_>`.

New `polyp` settings, whose defaults match the user's kitty config: `font_weight` 500, `bold_weight` 800, `bold_italic_weight` 900, and `italic_style` of `oblique` (default) or `italic`. Weights accept 100 to 900. They stay in the file, not in the settings panel.

- [ ] **Step 1: Write the failing tests:** `weights_parse_and_stay_in_range` (`polyp bold_weight 950` is an error, `800` is taken), `italic_style_is_oblique_or_italic`, and `attrs(&face, true, true)` has `Weight(900)` and `Style::Oblique`.
- [ ] **Step 2: Run** `cargo test -j 4 --test config --test render`. Expected: FAIL on the missing keys.
- [ ] **Step 3: Implement.** `Renderer::remeasure` already clears the shaped lines, so `set_font` needs no new cache handling.
- [ ] **Step 4: Run the Gate.**
- [ ] **Step 5: Compare on the virtual monitor** with kitty: normal, bold, italic and bold italic lines.
- [ ] **Step 6: Commit** `feat: choose font weights and the italic style`.
- [ ] **Step 7: Release 0.4.0** by `docs/RELEASING.md`, with the user's ok for each push.

---

## Phase 5: v0.5.0, desktop, search and links

### Task 14: Titles from programs

**Files:**
- Modify: `src/pty.rs`, `src/tabs.rs`, `src/main.rs`, `tests/tabs.rs`

**Interfaces:**
- Produces: `UserEvent::Title(u64, Option<String>)`; `Terminal.title: Option<String>`; `tabs::title(blocks: &BlockStore, program: Option<&str>, home: Option<&str>) -> String`; `tabs::clean_title(text: &str) -> Option<String>`.

Rules: at a prompt with marks, the cwd (a stale program title is ignored). A running command with a program title, that title. A running command without one, the command. No marks at all, the program title, then `koral`.

- [ ] **Step 1: Write the failing tests** in `tests/tabs.rs`:
  - `a_running_program_names_the_tab`, `the_prompt_ignores_a_stale_title`.
  - `a_shell_without_marks_uses_the_program_title`, `no_marks_and_no_title_is_koral`.
  - `clean_title("a\x1b]0;b\x07c") == Some("a]0;bc".to_string())`, `clean_title("\x07") == None`, and a 1000-character title is cut to 256.
- [ ] **Step 2: Run** `cargo test -j 4 --test tabs`. Expected: FAIL on the new argument.
- [ ] **Step 3: Implement.** Map `Event::Title` and `Event::ResetTitle` in `Listener::send_event`.
- [ ] **Step 4: Run the Gate, then commit** `feat: show the title a program asks for`.

### Task 15: Bell and urgency

**Files:**
- Modify: `src/pty.rs`, `src/tabs.rs`, `src/frame.rs`, `src/main.rs`, `tests/tabs.rs`, `tests/render.rs`

**Interfaces:**
- Produces: `UserEvent::Bell(u64)`; `Terminal.rang: bool`; `TabView.rang: bool`; `tabs::ring(active: bool, focused: bool) -> Ring { mark: bool, urgent: bool }`.

No sound: the user's kitty has the audio bell off. An inactive tab gets a `!` before its title, in the accent colour, so the state is not colour alone. An unfocused window asks for attention with `request_user_attention(Some(UserAttentionType::Informational))`. Selecting the tab clears the mark.

- [ ] **Step 1: Write the failing tests:** the four cases of `ring`, and `a_rung_tab_shows_a_mark` checks the strip text in `tests/render.rs`.
- [ ] **Step 2: Run** `cargo test -j 4 --test tabs --test render`. Expected: FAIL on the missing `ring`.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the Gate, then commit** `feat: mark tabs that ring`.
- [ ] **Step 5: Ask the user to check** that Hyprland shows urgency for an unfocused Koral window. The virtual monitor runs sway, so it cannot prove this.

### Task 16: Desktop notifications

**Files:**
- Create: `src/notify.rs`, `tests/notify.rs`
- Modify: `src/lib.rs`, `src/marks.rs`, `src/session.rs`, `src/pty.rs`, `src/main.rs`, `tests/marks.rs`

**Interfaces:**
- Produces: `Mark::Notify { title: String, body: String }`; `Session.notices: Vec<Notice>`, drained like `replies`; `UserEvent::Notify(u64, Notice)`; `notify::Limiter::allow(&mut self, now: Instant) -> bool`; `notify::clean(text: &str, limit: usize) -> String`; `notify::show(notice: &Notice)`.

Accepted forms: `OSC 777;notify;<title>;<body>` and `OSC 9;<body>`. An OSC 9 body that starts with digits and `;` is ConEmu progress, not a message, and stays with vte. Notifications work on the alternate screen, unlike block marks. Shown only when the window is unfocused or the tab inactive. At most one every 2 s, title capped at 256 characters, body at 1024.

- [ ] **Step 1: Write the failing tests:**
  - `tests/marks.rs`: both forms give `Mark::Notify`, `osc9_progress_is_not_a_notification`, `a_notification_split_across_reads_is_still_found`, and an unknown `777;foo` passes through as bytes.
  - `tests/notify.rs`: `the_limiter_allows_one_every_two_seconds`, `clean_drops_control_characters`, `clean_cuts_at_the_limit_on_a_character_boundary`.
- [ ] **Step 2: Run** `cargo test -j 4 --test marks --test notify`. Expected: FAIL on the missing variant and module.
- [ ] **Step 3: Implement.** `show` spawns `notify-send --app-name=koral --icon=koral -- <title> <body>` with `std::process::Command`, no shell, and waits on a thread so no zombie stays. A missing binary is ignored. Add `libnotify` to `optdepends`.
- [ ] **Step 4: Run the Gate, then commit** `feat: pass program notifications to the desktop`.

### Task 17: Search in the scrollback

**Files:**
- Create: `src/search.rs`, `tests/search.rs`
- Modify: `src/lib.rs`, `src/input.rs`, `src/view.rs`, `src/frame.rs`, `src/main.rs`, `tests/bindings.rs`

**Interfaces:**
- Consumes: `alacritty_terminal::term::search::{RegexSearch, RegexIter, Match}`, `Term::search_next`.
- Produces:

```rust
pub fn escape(query: &str) -> String // the query is plain text, not a regex
pub struct Search { pub query: String, pub current: Option<Match>, /* regex */ }
impl Search {
    pub fn set_query(&mut self, query: String);
    pub fn step<L: EventListener>(&mut self, term: &Term<L>, direction: Direction) -> Option<Match>;
    pub fn visible<L: EventListener>(&mut self, term: &Term<L>, first: Line, last: Line) -> Vec<Match>;
}
```

  `view::block_of(rows: &[Row], line: Line) -> Option<u32>` names the folded block that hides a line. `view::row_of(rows: &[Row], line: Line) -> Option<usize>`. `Action::Search`. `frame::draw_search(frame, rows, top, matches, current, layout, theme)` and `frame::draw_search_bar(frame, query, found, layout, theme)`.

Key: `ctrl+shift+s`, since `ctrl+shift+f` already folds. In the bar, typing edits the query, Enter goes to the next match upward, Shift+Enter downward, Esc closes. Search starts at the bottom and wraps. Lowercase queries ignore case, which `RegexSearch` does on its own. A bar with no match says `No matches`. The bar closes on a tab switch.

- [ ] **Step 1: Write the failing tests** in `tests/search.rs`:
  - `escape("a.b(c)") == r"a\.b\(c\)"`, and a query of `.` matches only a literal dot.
  - `finds_a_word_in_the_scrollback`: feed 200 lines with `needle` on line 3, assert the match line.
  - `steps_go_up_and_wrap`: two hits, three `step(Direction::Left)` calls land on the first hit again.
  - `lowercase_ignores_case`, `uppercase_is_exact`.
  - `anchors_do_not_break_a_match`: with block marks fed before `needle`, it is still found.
  - `a_match_in_a_fold_names_its_block`: a folded block hides the line, `view::row_of` is None and `view::block_of` is its id.
  - `no_match_is_none`, `an_empty_query_finds_nothing`.
  - `tests/bindings.rs`: `ctrl+shift+s` maps to `Action::Search`.
- [ ] **Step 2: Run** `cargo test -j 4 --test search --test bindings`. Expected: FAIL on the missing module.
- [ ] **Step 3: Implement `search.rs` and the `view` helpers.**
- [ ] **Step 4: Add the bar to `App`** as `search: Option<Search>`, handled like `settings` in `window_event`, including IME commits. On a step: if `row_of` is None, unfold `block_of` and recompute the rows, then set `tab.scroll` so the match sits in the middle of the view.
- [ ] **Step 5: Draw** matches at accent 0.25 and the current one at 0.6 with a border, so it is not colour alone.
- [ ] **Step 6: Run the Gate.**
- [ ] **Step 7: Check on the virtual monitor:** `seq 1 20000`, search `19999`, then a match in a folded command.
- [ ] **Step 8: Commit** `feat: search the scrollback`.

### Task 18: Clickable links

**Files:**
- Create: `src/links.rs`, `tests/links.rs`
- Modify: `src/lib.rs`, `src/frame.rs`, `src/main.rs`

**Interfaces:**
- Consumes: `Cell::hyperlink()`, `Term::regex_search_left`, `Term::regex_search_right`, `Point` from `point_under_cursor`.
- Produces: `links::Link { uri: String, range: Match }`; `links::at<L: EventListener>(term: &Term<L>, point: Point) -> Option<Link>`; `links::allowed(uri: &str) -> bool`; `links::open(uri: &str)`; `frame::draw_link(frame, rows, top, link, layout, theme)`.

Rules: OSC 8 first, then plain URLs. Ctrl+hover underlines the link, shows the pointer icon and shows the real target in a small bar at the bottom. Ctrl+click opens it. When an app has mouse reporting on, ctrl+shift+click opens it. Schemes allowed: `http`, `https`, `mailto`, `file`. Trailing `.,;:!?)` is not part of a plain URL.

- [ ] **Step 1: Write the failing tests** in `tests/links.rs`:
  - `finds_a_plain_url`, `drops_trailing_punctuation` (`see https://a.b/c.` ends at `c`), `keeps_a_url_across_a_wrapped_line`.
  - `finds_an_osc8_link`: feed `\x1b]8;;https://real.example\x1b\\click\x1b]8;;\x1b\\`, assert the URI is the real target and the range covers `click`.
  - `osc8_beats_the_text_it_shows`: visible text `https://fake.example` with target `https://real.example` gives the real target.
  - `no_link_on_plain_text`.
  - `allowed`: true for `https://a`, `mailto:a@b`, `file:///a`; false for `javascript:alert(1)`, `data:text/html,x`, `ssh://a`, `-x`, and an empty string.
- [ ] **Step 2: Run** `cargo test -j 4 --test links`. Expected: FAIL on the missing module.
- [ ] **Step 3: Implement.** `open` runs `xdg-open <uri>` with `std::process::Command`, no shell, and waits on a thread. It returns without spawning when `allowed` is false. Add `xdg-utils` to `optdepends`.
- [ ] **Step 4: Wire `main.rs`:** recompute the link in `cursor_moved` and on `ModifiersChanged`, and redraw when it changes. The click goes first in `press_terminal`.
- [ ] **Step 5: Run the Gate.**
- [ ] **Step 6: Check on the virtual monitor** with `ls --hyperlink=always` and a plain URL, using a stub `xdg-open` first in `PATH` that records its argument. Do not open a real browser.
- [ ] **Step 7: Commit** `feat: open links with ctrl and click`.
- [ ] **Step 8: Release 0.5.0** by `docs/RELEASING.md`, with the user's ok for each push.

---

## Phase 6: v1.0.0

### Task 19: Conformance

**Files:**
- Create: `docs/testing.md`
- Fix tasks follow from the findings; each gets a regression test first.

- [ ] **Step 1: Ask the user to install the tools.** `neovim`, `chafa` and `yazi` are in the official repositories, `vttest` was not found there, and `timg` was not found by `pacman -Ss`:

```bash
sudo pacman -S neovim chafa yazi
```

- [ ] **Step 2: Run each on the virtual monitor** and save a screenshot: `vttest` if available, `nvim` (undercurl, cursor shape, resize), `tmux`, `btop`, `fastfetch`, `yazi` image preview, `chafa -f kitty`, `kitten icat`.
- [ ] **Step 3: Check the stress cases:** live resize with folds open, a large `cat` in a background tab, an image in the scrollback.
- [ ] **Step 4: Record** pass or fail per program in `docs/testing.md`. Each failure becomes a test and a fix, then gets checked again.
- [ ] **Step 5: Commit** `docs: record terminal conformance`.

### Task 20: Performance

**Files:**
- Modify: `docs/testing.md`
- Create: `scripts/bench.sh`

**Interfaces:**
- Produces: `scripts/bench.sh <koral|kitty>`, which prints start time, 100 MB `cat` time and peak RSS.

- [ ] **Step 1: Write `scripts/bench.sh`.** The terminal starts a script as its shell that records a nanosecond timestamp, so the launch-to-shell time is measured the same way for both terminals. Then `cat` a 100 MB file made in a temporary directory and record the end time. `/usr/bin/time -v` gives peak RSS.
- [ ] **Step 2: Run it three times per terminal** on the virtual monitor.
- [ ] **Step 3: Compare with the spec's targets:** cold start under 100 ms, and no freeze while printing 100 MB. Press a key during `cat` and confirm that it shows up.
- [ ] **Step 4: Record the table** in `docs/testing.md`, marked as software rendering on the virtual monitor. Each missed target becomes a fix with a before and after number.
- [ ] **Step 5: Ask the user to judge** typing latency and video on the real Vega GPU. The virtual monitor cannot measure them.
- [ ] **Step 6: Commit** `docs: record performance against kitty`.

### Task 21: Release 1.0.0

**Files:**
- Modify: `README.md`, `docs/superpowers/specs/2026-10-31-koral-design.md`, `Cargo.toml`, `packaging/PKGBUILD`

- [ ] **Step 1: Wait for the daily-driving verdict** from the user and their girlfriend, and fix what hurts. Each fix gets a regression test first.
- [ ] **Step 2: Update** the README shortcuts, config keys and the "without sharky" notes. Mark the spec's goals as met or list what is still open.
- [ ] **Step 3: Run the Gate.**
- [ ] **Step 4: Release 1.0.0** by `docs/RELEASING.md`, with the user's ok for each push.
- [ ] **Step 5: Leave the `SUPER + Return` switch to the user.** It is their decision and their config.
