# NixOS module for chatmixd. Importable directly (no flakes required):
#
#   imports = [ /path/to/chatmixd/nix/module.nix ];
#   services.chatmixd.enable = true;
#
# or via the flake's `nixosModules.default`, in which case you may want to
# set `services.chatmixd.package` from the flake's package output.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.chatmixd;
in
{
  options.services.chatmixd = {
    enable = lib.mkEnableOption "chatmixd, the SteelSeries ChatMix daemon";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ./package.nix { };
      defaultText = lib.literalExpression "pkgs.callPackage ./package.nix { }";
      description = "The chatmixd package to use.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    # Ships 99-chatmixd.rules: uaccess ACL on SteelSeries hidraw nodes for
    # the active graphical session, so the daemon can run unprivileged.
    services.udev.packages = [ cfg.package ];

    # The unit is declared here rather than picked up from the package's
    # lib/systemd/user, because NixOS does not honor [Install] sections of
    # packaged units — the service would never be enabled.
    systemd.user.services.chatmixd = {
      description = "SteelSeries ChatMix daemon";
      after = [
        "pipewire.service"
        "wireplumber.service"
      ];
      partOf = [ "pipewire.service" ];
      wantedBy = [ "default.target" ];
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/chatmixd";
        Restart = "always";
        RestartSec = 2;
      };
    };
  };
}
