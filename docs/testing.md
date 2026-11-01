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
| Koral | 81, 112, 120 ms | 1.40, 1.58, 1.40 s | 178, 187, 179 MiB |
| kitty | 370, 146, 146 ms | 1.25, 1.25, 1.24 s | 146, 148, 148 MiB |

The 100 ms start target holds in one run of three on software rendering, and Koral
starts faster than kitty in all three. Typing latency and video need a real GPU.

Inside the process a warm start takes 75 ms to the first tab: 8 ms for the GPU
setup, 18 ms to list the fonts, and 48 ms for the text pipelines and the first
measure, which software rendering makes slower.

The real miss is a start with the fonts out of the page cache: listing the 1074
faces on the test machine took 2.4 s, once, before the window appeared. The fix
is to load the configured font first and the fallbacks behind it.

## Left to a person

- Accents through a dead key on ABNT2: the virtual keyboard cannot compose them.
- Urgency on Hyprland: the virtual monitor runs sway.
- Typing latency and video on a real GPU.
- A fresh install on a second machine.
