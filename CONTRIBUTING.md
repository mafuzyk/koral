# Contributing to Koral

Koral is a Linux-only terminal emulator built in Rust (edition 2024, MSRV 1.88,
`GPL-3.0-only`). Development targets Wayland first; the current stable release is `1.2.1`.
All code, docs, tests, filenames, and commits are written in English.

## Development setup

The Nix flake is the supported development environment. From the repository root:

```sh
nix develop
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --release --locked
cargo test --doc
```

`nix develop` provides `rustc`, `cargo`, `rustfmt`, `clippy`, `pkg-config`, and the runtime
libraries (`libxkbcommon`, `wayland`, `fontconfig`, `vulkan-loader`, `libGL`). Without Nix,
install a Rust toolchain of at least 1.88 and those libraries yourself. To run the GUI:

```sh
cargo build --release --locked
./target/release/koral
```

Requirements before opening a pull request:

- `cargo fmt --check` passes.
- `cargo clippy --all-targets -- -D warnings` passes with zero warnings.
- `cargo test --release --locked` passes. Doc tests are checked too. Run CI locally with
  `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test --release --locked`, and `cargo test --doc`.
- New behavior in the VT, graphics, IPC, or config paths gets a targeted test in `tests/`.
- Changes to the vendored VT parser stay minimal and are reflected in both
  [vendor/alacritty_terminal/KORAL-PATCH.md](vendor/alacritty_terminal/KORAL-PATCH.md) and
  `vendor/alacritty_terminal/koral.patch`.

## Benchmark reproduction

Use an isolated graphical session, never the real desktop, for end-to-end benchmarks.
Build or obtain the release binary separately, then pin its absolute path:

```sh
KORAL_BIN=/absolute/path/to/koral scripts/bench.sh --terminals koral,kitty \
  --runs 5 --config NONE --json /path/to/bench.json
```

The harness generates a shared 100 MB payload and isolates configuration per run in
`NONE` mode. It records launch-to-shell time, producer-side output time and peak RSS
(`VmHWM`). Producer time is not end-to-end rendering time. The latency helper requires
both confirmed PTY input and a captured frame change. An unavailable or timed-out
sample is not a latency result.

Record pinned versions, compositor, resolution, refresh rate, GPU backend and host
load. Compare like-for-like runs and report distributions, not a single sample. The
historical README numbers predate these checks and are not a comparative baseline.

## Release channels

| Channel | Contents | Audience |
| --- | --- | --- |
| `1.2.x` stable | Bugfixes and security backports only, no config breaks | Daily users |
| `1.2.1` | Current stable release, tagged `v1.2.1` | Recommended |
| `koral-git` (AUR) | Builds the `2.0.0-alpha.N` development line | Early testers and contributors |
| `koral-beta` (AUR) | Feature-frozen `2.0.0-beta.N` / `2.0.0-rc.N` builds | Beta testers |
| `koral-bin` (AUR) | Prebuilt `1.2.x` stable binary, default Nix `koral-bin` package | Daily users |
| Nix `koral` / `nix run github:mafuzyk/koral` | Source build, currently pinned to `1.2.1` | Nix users |

The alpha line accepts breaking config and protocol changes; beta takes only fixes after
feature freeze, and release candidates take regression fixes only. Stable fixes that do not
break configuration are backported. Never target `main` with a change that belongs to an
alpha-only feature without discussing it first.

## Pull requests

Before opening a PR:

1. Create a branch from `main` with a short descriptive name.
2. Keep one logical change per commit and follow the commit convention below.
3. Run the full check list above and add or update tests for behavior changes.
4. Fill in [.github/PULL_REQUEST_TEMPLATE.md](.github/PULL_REQUEST_TEMPLATE.md). Link any
   related issue and state clearly how you verified the change.
5. Use the structured [bug report](.github/ISSUE_TEMPLATE/bug_report.yml) or
   [feature request](.github/ISSUE_TEMPLATE/feature_request.yml) template for new issues.
   Report security vulnerabilities privately as described in [SECURITY.md](SECURITY.md),
   never in a public issue.

CI is expected to run formatting, strict Clippy, the release-mode test suite, and doc
tests on pull requests. A PR is merged after CI is green and review feedback is resolved.

## Commit convention

Use Conventional Commits in English, without `Co-authored-by` or other AI attribution
trailers:

```text
<type>(optional scope): <imperative summary>

feat(graphics): support unicode placeholders in placements
fix(session): preserve image anchors on erase
perf(pty): batch burst ingestion across reads
docs: add security policy and contributing guide
test(ipc): cover socket cleanup and 0600 permissions
ci: add pr checks and prerelease-aware releases
chore: release 1.2.1
```

Common types are `feat`, `fix`, `perf`, `docs`, `test`, `refactor`, `ci`, `build`, and
`chore`. Keep the summary lowercase and under roughly 72 characters. Release commits bump
`Cargo.toml`, `flake.nix`, package checksums, and [CHANGELOG.md](CHANGELOG.md) together.

## License

Contributions are accepted under the repository's `GPL-3.0-only` license, see
[LICENSE](LICENSE).
