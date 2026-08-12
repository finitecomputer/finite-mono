{
  config,
  lib,
  ...
}:
let
  cfg = config.finite.secrets;
  hostName = config.networking.hostName;

  activeOnHost = secret: secret.scope == [ ] || builtins.elem hostName secret.scope;

  sopsManagedFiles = lib.filterAttrs (_name: secret: activeOnHost secret) cfg.files;

  toSopsSecret =
    name: secret:
    {
      sopsFile = secret.sopsFile;
      format = secret.sopsFormat;
      owner = secret.owner;
      group = secret.group;
      mode = secret.mode;
      path = secret.destinationPath;
      restartUnits = secret.restartUnits;
      reloadUnits = secret.reloadUnits;
    }
    // lib.optionalAttrs (secret.sopsKey != null) {
      key = secret.sopsKey;
    };

  secretModule =
    { name, config, ... }:
    {
      options = {
        scope = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ hostName ];
          description = "Production hosts this secret entry is active on. Empty means every host.";
        };

        sopsFile = lib.mkOption {
          type = lib.types.path;
          example = "../secrets/shared/metrics-remote-write.env";
          description = "Tracked encrypted SOPS source file.";
        };

        destinationPath = lib.mkOption {
          type = lib.types.str;
          default = "/run/secrets/finite/${name}";
          description = "Runtime path consumers should read after this entry moves to SOPS.";
        };

        path = lib.mkOption {
          type = lib.types.str;
          readOnly = true;
          default = config.destinationPath;
          description = "Resolved SOPS-managed runtime path for service modules to consume.";
        };

        owner = lib.mkOption {
          type = lib.types.str;
          default = "root";
          description = "Owner for the runtime secret file.";
        };

        group = lib.mkOption {
          type = lib.types.str;
          default = "root";
          description = "Group for the runtime secret file.";
        };

        mode = lib.mkOption {
          type = lib.types.str;
          default = "0600";
          description = "Octal mode for the runtime secret file.";
        };

        kind = lib.mkOption {
          type = lib.types.enum [
            "env"
            "opaque"
          ];
          description = "Consumer-facing file kind for validation.";
        };

        requiredEnvNames = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          description = "Required variable names for env-file entries. Values are never represented.";
        };

        consumers = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          description = "Systemd units, container definitions, or recovery jobs that consume this entry.";
        };

        restartUnits = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          description = "Units sops-nix should restart when the decrypted material changes.";
        };

        reloadUnits = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          description = "Units sops-nix should reload when the decrypted material changes.";
        };

        sopsFormat = lib.mkOption {
          type = lib.types.enum [
            "binary"
            "dotenv"
            "ini"
            "json"
            "yaml"
          ];
          default = "binary";
          description = "SOPS source format. Binary keeps whole-file env migrations byte-for-byte.";
        };

        sopsKey = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Optional key inside structured SOPS files. Use an empty string for whole-file YAML/JSON emission.";
        };
      };
    };
in
{
  options.finite.secrets.files = lib.mkOption {
    type = lib.types.attrsOf (lib.types.submodule secretModule);
    default = { };
    description = "Values-free production secret contract entries.";
  };

  config = {
    sops.age.keyFile = lib.mkDefault "/var/lib/sops-nix/${hostName}.agekey";
    sops.secrets = lib.mapAttrs toSopsSecret sopsManagedFiles;

    assertions = lib.mapAttrsToList (name: secret: {
      assertion = builtins.match "[0-7][0-7][0-7][0-7]" secret.mode != null;
      message = "finite.secrets.files.${name}: mode must be four octal digits";
    }) cfg.files;
  };
}
