# Koral

A Rust terminal with native sharky shell support. It understands
[sharky](https://github.com/mafuzyk/sharky-shell) commands as units: each one can
fold, shows its status and duration, and can be copied or jumped to on its own.
Tabs are native, with no tmux involved.

![Koral with a folded command, an image, and command status on the right](assets/screenshot.png)

Koral runs on Linux under Wayland and is developed on Hyprland.

## Install

```sh
yay -S koral-bin
```

To build from source:

```sh
cargo build --release
./target/release/koral
```

The default font is IosevkaTerm Nerd Font Mono.

## Shortcuts

| Keys | Action |
| --- | --- |
| `ctrl+shift+t` | New tab |
| `ctrl+shift+w` | Close tab |
| `ctrl+tab`, `ctrl+shift+tab` | Next and previous tab |
| `alt+1` to `alt+9` | Go to a tab |
| `ctrl+shift+c`, `ctrl+shift+v` | Copy and paste |
| `ctrl+shift+z`, `ctrl+shift+x` | Previous and next prompt |
| `ctrl+shift+f` | Fold or open a command |
| `ctrl+shift+g` | Copy a command's output |
| `ctrl+shift+s` | Search the scrollback |
| `ctrl+=`, `ctrl+-`, also with `shift`; `ctrl+shift+0` | Zoom in, out and back |
| `shift+pageup`, `shift+pagedown` | Scroll a page |
| `ctrl+shift+,` | Settings |

Click the marker to the left of a command to fold it. Selecting text copies it to
the primary selection, and the middle button pastes it. Double-click selects a
word, triple-click a line, and `ctrl+alt` with a drag a block.

In the search bar, Enter goes to the next match upward, `shift+enter` downward, and
Esc closes it. Lowercase queries ignore case.

Hold `ctrl` over a link to see its real target, and `ctrl`+click to open it. When a
program uses the mouse, `ctrl+shift`+click opens it. Only `http`, `https`, `mailto`
and `file` links open, through `xdg-open`.

## Configuration

Use the gear in the tab strip, or edit `~/.config/koral/config.sharky`:

```
polyp font "IosevkaTerm Nerd Font Mono"
polyp font_size 13.5
polyp font_weight 500
polyp bold_weight 800
polyp bold_italic_weight 900
polyp italic_style oblique
polyp opacity 0.8
polyp padding 8
polyp shell /usr/bin/sharky
polyp matugen on
```

Weights go from 100 to 900, and `italic_style` is `oblique` or `italic`. The file
uses sharky's config language. Colours come from `theme.sharky`, with
`wave terminal.background @abyss` and the like.

With `matugen on`, Koral adds its template to your matugen config and takes its
colours from the next wallpaper change.

## Without sharky

Koral starts sharky when it finds it, and `$SHELL` otherwise. Any shell works as a
plain terminal with tabs, search, links, images and video. Folds, status and prompt
jumps need the marks sharky prints.

## Images and video

Koral speaks the kitty graphics protocol, so `mpv --vo=kitty` and other programs
that use it work.

Programs can set the tab title, ring the bell and send desktop notifications with
`OSC 777` or `OSC 9`. Notifications need `libnotify`, and links need `xdg-utils`.

## Licence

GPL-3.0-only. See [LICENSE](LICENSE).
