# koral-bin is the default package and points at the current release tarball.
# For a new release, bump Cargo.toml, `version`, `releaseVersion` and the tarball
# hash together, refresh `cargoHash` (the lock changes with the bump), tag, and
# let CI publish the tarball.
{
  description = "A Rust terminal with native sharky shell support";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      systems = [ "x86_64-linux" ];
      forAllSystems = f: lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));
      version = "1.2.0";
      releaseVersion = "1.2.0";
    in
    {
      packages = forAllSystems (pkgs: rec {
        koral = pkgs.rustPlatform.buildRustPackage {
          pname = "koral";
          inherit version;
          src = self;

          cargoHash = "sha256-Orj0sVH5p5etikLbNXI680FYsmlvX6ihijWlr8F8Pao=";

          nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];
          buildInputs = with pkgs; [ libxkbcommon wayland fontconfig vulkan-loader libGL ];

          postInstall = ''
            install -Dm644 assets/koral.desktop $out/share/applications/koral.desktop
            install -Dm644 assets/koral.svg $out/share/icons/hicolor/scalable/apps/koral.svg
            wrapProgram $out/bin/koral \
              --prefix PATH : ${lib.makeBinPath (with pkgs; [ libnotify xdg-utils fontconfig ])} \
              --prefix LD_LIBRARY_PATH : /run/opengl-driver/lib:${lib.makeLibraryPath (with pkgs; [ libxkbcommon wayland vulkan-loader libGL ])}
          '';

          meta = {
            description = "A Rust terminal with native sharky shell support";
            homepage = "https://github.com/mafuzyk/koral";
            license = lib.licenses.gpl3Only;
            mainProgram = "koral";
            platforms = [ "x86_64-linux" ];
          };
        };
        koral-bin = pkgs.stdenv.mkDerivation {
          pname = "koral-bin";
          version = releaseVersion;
          src = pkgs.fetchurl {
            url = "https://github.com/mafuzyk/koral/releases/download/v${releaseVersion}/koral-x86_64-linux.tar.gz";
            hash = "sha256-kn82zJa1yqEdllcKi9VDGYWaqxbgpnqN89shcXqI06s=";
          };

          nativeBuildInputs = with pkgs; [ autoPatchelfHook makeWrapper ];
          buildInputs = with pkgs; [ libxkbcommon wayland vulkan-loader libGL stdenv.cc.cc.lib ];

          unpackPhase = ''
            tar xzf $src
          '';
          installPhase = ''
            install -Dm755 koral $out/bin/koral
            install -Dm644 koral.desktop $out/share/applications/koral.desktop
            install -Dm644 koral.svg $out/share/icons/hicolor/scalable/apps/koral.svg
            install -Dm644 LICENSE $out/share/licenses/koral-bin/LICENSE
            wrapProgram $out/bin/koral \
              --prefix PATH : ${lib.makeBinPath (with pkgs; [ libnotify xdg-utils fontconfig ])} \
              --prefix LD_LIBRARY_PATH : /run/opengl-driver/lib:${lib.makeLibraryPath (with pkgs; [ libxkbcommon wayland vulkan-loader libGL ])}
          '';

          meta = {
            description = "A Rust terminal with native sharky shell support (prebuilt release binary)";
            homepage = "https://github.com/mafuzyk/koral";
            license = lib.licenses.gpl3Only;
            mainProgram = "koral";
            platforms = [ "x86_64-linux" ];
            sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
          };
        };
        default = koral-bin;
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [ rustc cargo rustfmt clippy pkg-config makeWrapper ];
          buildInputs = with pkgs; [ libxkbcommon wayland fontconfig vulkan-loader libGL ];
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
