# Release order: koral-bin currently points at the v1.0.3 tarball so the flake stays buildable.
# Bump Cargo.toml and the source `version` to the release version, tag, and let CI publish the tarball.
# Then bump `releaseVersion` and its hash together and point `default` at `koral-bin`.
{
  description = "A Rust terminal with native sharky shell support";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      systems = [ "x86_64-linux" ];
      forAllSystems = f: lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));
      version = "1.0.3";
      releaseVersion = "1.0.3";
    in
    {
      packages = forAllSystems (pkgs: rec {
        koral = pkgs.rustPlatform.buildRustPackage {
          pname = "koral";
          inherit version;
          src = self;

          cargoHash = "sha256-Dwf0FVf98yrctbz0Vezj3pWN4T8FP2bgUaI6kRNpCVI=";

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
            hash = "sha256-TFxxBthHhWZrQViDuPEJkz1s1dbyvgnxDWrysa6gA5Q=";
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
        # Flip default to koral-bin once the next tag's hash is pinned.
        default = koral;
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
