# Testing

Results for 1.0.0, from a headless sway session with software rendering on
2026-11-01. Real GPU behaviour is not covered here.

## Conformance

| Program | Result | Notes |
| --- | --- | --- |
| vim | Pass | Colours, line numbers, insert mode cursor (`t_SI`) |
| tmux | Pass | Split, status line |
| btop | Pass | Box drawing, braille graphs, colours |
| fastfetch | Pass | Logo and palette |
| `kitten icat` | Pass after a fix | Waited 10 s on a short device reply; now 24 ms |
| `ls --hyperlink` | Pass | Opens the `file://` target |
| `mpv --vo=kitty` | Pass | Checked at 0.1.0, not repeated |
| neovim | Not run | Not installed on the test machine |
| chafa | Not run | Not installed on the test machine |
| yazi | Not run | Not installed on the test machine |
| vttest | Not run | Not in the official repositories |

## Stress

| Case | Result |
| --- | --- |
| Resize with a folded command | Pass: the fold and the view hold |
| 60 MB of output in a background tab | Pass: the active tab stays responsive |
| `cat` of 100 MB, opening a tab during it | Pass: the new tab answers at once |
| An image in the scrollback | Pass: it scrolls with the text and folds with its command |
| Search for a match inside a fold | Pass: the fold opens and the match is centred |

## Performance

`scripts/bench.sh <koral|kitty>`, three runs each. Start is launch to shell, cat
is 100 MB of base64, peak is the terminal's RSS at the end.

| Terminal | Start | Cat | Peak |
| --- | --- | --- | --- |
| Koral | 39, 43, 39 ms | 1.38, 1.41, 1.40 s | 175, 176, 176 MiB |
| kitty | 370, 146, 146 ms | 1.25, 1.25, 1.24 s | 146, 148, 148 MiB |

A start used to list every font on the system. Koral now keeps the files of the
four styles it draws in `~/.cache/koral/fonts`, loads those, and lists the rest
behind the first frame for fallback glyphs.

| Start | Before | After |
| --- | --- | --- |
| Warm | 77 to 80 ms | 40 to 45 ms |
| Fonts out of the page cache | 3222 to 3555 ms | 98 to 100 ms |

The target of 100 ms holds in both. The first start ever, or the first after a
font change, still lists every font: 3.3 s with a cold cache on the test machine.
Glyphs from fallback fonts appear once the list is in.

## Left to a person

- Accents through a dead key on ABNT2: the virtual keyboard cannot compose them.
- Urgency on Hyprland: the virtual monitor runs sway.
- Typing latency and video on a real GPU.
- A fresh install on a second machine.
