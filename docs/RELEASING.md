# Releasing Koral

Every push, GitHub release and AUR push is a deliberate step: get the maintainer's
ok before each one.

## 1. Bump the version

Set the version in `Cargo.toml` and `pkgver` in `packaging/PKGBUILD`, reset
`pkgrel` to 1, and run `cargo build` to update `Cargo.lock`.

## 2. Final checks

```sh
systemd-run --user --scope --quiet -p MemoryMax=3G -- cargo test -j 4
systemd-run --user --scope --quiet -p MemoryMax=3G -- cargo clippy -j 4 --all-targets
systemd-run --user --scope --quiet -p MemoryMax=3G -- cargo build --release --locked -j 4
./target/release/koral --version
```

## 3. Tag and publish

```sh
git commit -am "chore: release <version>"
git tag -a v<version> -m "koral <version>"
git push origin main v<version>
```

The tag starts `.github/workflows/release.yml`, which tests, builds and uploads
`koral-x86_64-linux.tar.gz` to the release. Binaries come from there, not from
this machine: its build flags target the local CPU, so they may not run elsewhere.
Wait for it with `gh run watch`.

## 4. Build the package

The checksum comes from the release's tarball, so this runs after the workflow.

```sh
cd packaging
updpkgsums
makepkg -f
makepkg --printsrcinfo > .SRCINFO
pacman -Qlp koral-bin-<version>-1-x86_64.pkg.tar.zst
```

The package holds:

```
usr/bin/koral
usr/share/applications/koral.desktop
usr/share/icons/hicolor/scalable/apps/koral.svg
usr/share/licenses/koral-bin/LICENSE
```

## 5. AUR

```sh
git clone ssh://aur@aur.archlinux.org/koral-bin.git aur-koral
cp packaging/PKGBUILD packaging/.SRCINFO aur-koral/
cd aur-koral
git add PKGBUILD .SRCINFO
git commit -m "upgpkg: koral-bin <version>"
git push
```

Then commit the updated `packaging/` files and push them to GitHub.
