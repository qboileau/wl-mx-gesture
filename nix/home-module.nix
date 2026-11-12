self:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.wl-mx-gesture;
  toml = pkgs.formats.toml { };
in
{
  options.services.wl-mx-gesture = {
    enable = lib.mkEnableOption "wl-mx-gesture user service (hidraw access still needs the udev rule from the NixOS module)";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "wl-mx-gesture.packages.\${system}.default";
      description = "The wl-mx-gesture package to use.";
    };

    settings = lib.mkOption {
      inherit (toml) type;
      default = { };
      example = lib.literalExpression ''
        {
          bindings.swipe.left.hyprland = "workspace e-1";
          bindings.hold.wheel = "main";
          wheel.main.slots = [
            { label = "Close"; icon = "window-close"; action.hyprland = "killactive"; }
          ];
        }
      '';
      description = ''
        Written to {file}`$XDG_CONFIG_HOME/wl-mx-gesture/config.toml`.
        When empty, the built-in default configuration is used.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];

    xdg.configFile."wl-mx-gesture/config.toml" = lib.mkIf (cfg.settings != { }) {
      source = toml.generate "wl-mx-gesture-config.toml" cfg.settings;
    };

    systemd.user.services.wl-mx-gesture = {
      Unit = {
        Description = "Logitech MX Master gesture button daemon";
        PartOf = [ "graphical-session.target" ];
        After = [ "graphical-session.target" ];
        X-Restart-Triggers = lib.optional (cfg.settings != { }) config.xdg.configFile."wl-mx-gesture/config.toml".source;
      };
      Service = {
        ExecStart = lib.getExe cfg.package;
        Restart = "on-failure";
      };
      Install.WantedBy = [ "graphical-session.target" ];
    };
  };
}
