# Declarative start-down gate for a host being brought up with imported
# state. While `finite.importMode.enable` is true, the listed units are not
# wanted at boot: the host comes up with sshd, postgres, caddy disabled,
# and monitoring only, state is imported and verified offline, then the
# go-live closure flips the option and starts the product stack.
#
# The gate only removes boot-time wants; nothing here masks by force. Verify
# with `systemctl list-units --state=running` after the import-mode boot.
{
  config,
  lib,
  ...
}:
{
  options.finite.importMode = {
    enable = lib.mkEnableOption "start-down import mode (product units do not boot)";
    units = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = ''
        Unit names (service, timer, or synthetic podman units) that must not
        start while import mode is enabled.
      '';
    };
  };

  config = lib.mkIf config.finite.importMode.enable {
    systemd.units = lib.listToAttrs (
      map (unit: {
        name = unit;
        value = {
          wantedBy = lib.mkForce [ ];
        };
      }) config.finite.importMode.units
    );
  };
}
