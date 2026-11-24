# Security Policy

Koral is a Linux terminal emulator. It runs programs chosen by the user with the user's
own privileges; it is not a sandbox and does not isolate child processes. This policy
describes the trust boundaries Koral enforces and how to report a vulnerability privately.

## Supported versions

| Version | Status | Security fixes |
| --- | --- | --- |
| `1.2.1` (stable `1.2.x`) | Current stable release | Yes |
| `2.0.0-alpha.N` prereleases | Early testing, not for production | Latest alpha only, best effort |
| `1.2.0` and older | End of life | No |

Security fixes for `1.2.x` are kept compatible with the stable configuration format and
backported where safe. Breaking changes land in the `2.0.0-alpha.N` line instead. See
[CHANGELOG.md](CHANGELOG.md) for release contents and [CONTRIBUTING.md](CONTRIBUTING.md)
for the release channels.

## Reporting a vulnerability

Use GitHub Private Vulnerability Reporting for `mafuzyk/koral`:

1. Open <https://github.com/mafuzyk/koral/security/advisories/new> and choose
   **Report a vulnerability**.
2. Include the affected version (`koral --version`), a minimal reproducer, the impact you
   can demonstrate, and whether the issue needs a terminal session, an SSH connection, or a
   crafted escape sequence.
3. Do **not** open a public issue, discussion, or pull request for an unfixed vulnerability.
   If Private Vulnerability Reporting is unavailable for this repository, contact the
   maintainer privately through GitHub first and wait for an acknowledgement before
   disclosing details publicly.
4. Keep the report private until the coordinated disclosure date agreed in the advisory.

### Response timeline

These are the project's response targets, not a record of past response times.

| Stage | Target |
| --- | --- |
| Acknowledge receipt | Within 3 business days |
| Initial triage and severity assessment | Within 5 business days |
| Status update while a report is open | At least every 14 days |
| Fix or mitigation for critical issues | Within 7 days |
| Fix or mitigation for high-severity issues | Within 30 days |
| Fix or mitigation for moderate issues | Within 90 days |
| Low-severity issues | Best effort, tracked in the advisory |
| Coordinated public disclosure | By mutual agreement, no later than 90 days after triage unless the reporter agrees to extend |

A fix for an unsupported version is not guaranteed; upgrading to the current stable or
latest alpha is the recommended mitigation.

## Trust boundaries

### PTY output and escape sequences

Everything printed into a pane is untrusted. That includes local programs, output copied
from a remote `ssh` session, and text from files merely displayed by a TUI. Koral parses
OSC 133 / OSC 7 marks, OSC 9 / OSC 777 notifications, and Kitty graphics from that stream,
but never treats their contents as commands.

- Parser state machines are bounded: an unterminated or oversized sequence is dropped and
  the following valid VT bytes are parsed normally. Current caps include `MAX_MARK` (64 KiB),
  `MAX_STATUS` (32 bytes), and `MAX_GRAPHICS` (2 MiB) in `src/marks.rs`, plus `MAX_LINE`
  (64 KiB) for IPC requests. See [tests/marks.rs](tests/marks.rs).
- OSC 52 clipboard **reads** are off by default (`clipboard_read off` in
  `~/.config/koral/config.sharky`). Do not enable them in a session that prints untrusted
  output, because the program that prints a read request can ask Koral to put clipboard text
  into the PTY.
- Titles, notification text, and command marks are data displayed to the user. A hostile
  program can still spoof a prompt marker, a title, or a notification; Koral does not
  authenticate marks emitted by the child process.
- Clickable URLs and local paths are restricted to `http`, `https`, `mailto`, and `file`,
  and are opened with `xdg-open` only after an explicit click. Koral does not open links
  found in untrusted output automatically.

### Kitty graphics

Kitty graphics commands travel in APC payloads from the PTY and are untrusted.

- Only direct transmission (`t=d`) is accepted. Paths (`t=f`) and shared-memory transfers
  (`t=s`) are rejected with `ENOTSUPPORTED:only direct transmission`; Koral must never read
  a local file or shared-memory segment because a printed escape sequence named it. See
  [src/graphics.rs](src/graphics.rs).
- Decoded data is bounded: 64 MiB per image, 128 MiB per transfer, 320 MiB total store,
  and 1024 cells per placement. Undecodable or oversized payloads are rejected rather than
  allocating without limit. See [tests/graphics.rs](tests/graphics.rs).
- `KORAL_GRAPHICS_LOG=<path>` and `KORAL_FRAME_LOG=<path>` are opt-in diagnostics that write
  command keys, payload lengths, and frame timings to a caller-selected file. They are
  intended for trusted debugging sessions and may record sensitive payload metadata; do not
  enable them while running untrusted programs.

### IPC socket and remote control

`koral ctl` talks to a running window over a Unix domain socket at
`$XDG_RUNTIME_DIR/koral-<pid>.sock`, with permissions set to `0600` in
[src/ipc.rs](src/ipc.rs). The socket is exported to child processes as `KORAL_SOCKET`.

- Access to that socket is equivalent to control of the window: accepted requests can open
  tabs and run commands, send text into the focused pane, change themes, and reload config.
  Keep `$XDG_RUNTIME_DIR` private to your user and do not expose the socket to containers,
  other users, or untrusted processes.
- A stale `koral-*.sock` left by a killed window is ignored by clients that cannot connect
  to it; a new server unlinks and replaces its own path at bind time.
- Over forwarded SSH, the `0600` file mode is **not** sufficient: the request crosses a host
  boundary. The planned `koral +ssh` design reverse-forwards the control socket and adds a
  per-invocation auth token in `KORAL_CTL_TOKEN`; a forwarded control request with a missing
  or invalid token must be rejected. This token check is a `2.0.0-alpha.4` design requirement
  and is **not yet implemented** in `1.2.1`; until then, do not forward the control socket to
  an environment you do not trust.
- Config files and environment variables such as `KORAL_SHELL` come from the local
  user and are trusted. `~/.config/koral/config.sharky` is reloaded while Koral runs, so
  write access to it is equivalent to configuration control of the terminal.
- Vendored VT changes are tracked in
  [vendor/alacritty_terminal/KORAL-PATCH.md](vendor/alacritty_terminal/KORAL-PATCH.md) and
  [vendor/alacritty_terminal/koral.patch](vendor/alacritty_terminal/koral.patch).

## Out of scope

- Koral cannot make a malicious program stop printing misleading output, asking the user to
  paste a command, or using the privileges it already has.
- Physical access to an unlocked session, a compromised compositor or GPU driver, and
  vulnerabilities in programs run inside Koral are outside this policy.
- Packaging issues in third-party channels should also be reported to that channel's
  maintainers; the GitHub advisory flow covers the upstream Koral source and releases.
