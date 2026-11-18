{
  description = "A Rust terminal with native sharky shell support";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      systems = [ "x86_64-linux" ];
      forAllSystems = f: lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));
      version = "1.0.3";
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
