self:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.wl-mx-gesture;
in
{
  options.programs.wl-mx-gesture = {
    enable = lib.mkEnableOption "wl-mx-gesture: udev access to Logitech HID++ devices and a user service";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "wl-mx-gesture.packages.\${system}.default";
      description = "The wl-mx-gesture package to use.";
    };

    service = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Start the daemon with the graphical session. Disable when the home-manager module manages it.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    services.udev.packages = [ cfg.package ];

    systemd.user.services.wl-mx-gesture = lib.mkIf cfg.service {
      description = "Logitech MX Master gesture button daemon";
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      wantedBy = [ "graphical-session.target" ];
      serviceConfig = {
        ExecStart = lib.getExe cfg.package;
        Restart = "on-failure";
      };
    };
  };
}
