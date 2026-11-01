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
gh release create v<version> --generate-notes
```

## 4. Build the package

The checksum comes from the tag's tarball, so this runs after the tag is public.

```sh
cd packaging
updpkgsums
makepkg -f
makepkg --printsrcinfo > .SRCINFO
pacman -Qlp koral-<version>-1-x86_64.pkg.tar.zst
```

The package holds:

```
usr/bin/koral
usr/share/applications/koral.desktop
usr/share/icons/hicolor/scalable/apps/koral.svg
usr/share/licenses/koral/LICENSE
```

## 5. AUR

```sh
git clone ssh://aur@aur.archlinux.org/koral.git aur-koral
cp packaging/PKGBUILD packaging/.SRCINFO aur-koral/
cd aur-koral
git add PKGBUILD .SRCINFO
git commit -m "upgpkg: koral <version>"
git push
```

Then commit the updated `packaging/` files and push them to GitHub.
