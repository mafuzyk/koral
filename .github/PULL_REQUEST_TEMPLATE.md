## Summary

<!-- What changed and why? Keep it short. -->

## Related issue

<!-- Closes #... or "None". Do not link an unfixed security report here. -->

## Type of change

- [ ] Bug fix (compatible with `1.2.x`)
- [ ] New feature or breaking change (alpha/beta line)
- [ ] Performance improvement
- [ ] Documentation
- [ ] Test or CI maintenance

## Verification

List the exact commands you ran and their results:

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --release --locked`
- [ ] Manual verification (describe session type, compositor, and GPU backend below)

<!-- Add new tests or explain why existing tests cover the change. -->

## Environment

- Koral version (`koral --version`):
- Session type (Wayland/X11) and compositor:
- GPU backend:
- `TERM` and font family:

## Checklist

- [ ] Commits follow the Conventional Commits convention in CONTRIBUTING.md.
- [ ] Tests were added or updated for the changed behavior.
- [ ] Vendored VT changes, if any, are documented in `vendor/alacritty_terminal/KORAL-PATCH.md` and `vendor/alacritty_terminal/koral.patch`.
- [ ] CHANGELOG.md is updated when the change is user-visible.
- [ ] No credentials, personal data, or AI attribution trailers are included.
