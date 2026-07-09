{
  description = "SteelSeries ChatMix daemon for Linux (PipeWire/PulseAudio)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems =
        f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: rec {
        chatmixd = pkgs.callPackage ./nix/package.nix { };
        default = chatmixd;
      });

      overlays.default = final: prev: {
        chatmixd = final.callPackage ./nix/package.nix { };
      };

      nixosModules.default = ./nix/module.nix;
    };
}
