{
  config,
  lib,
  ...
}:
let
  cfg = config.finite.secrets;
  hostName = config.networking.hostName;

  activeOnHost = secret: secret.scope == [ ] || builtins.elem hostName secret.scope;

  sopsBackedFiles = lib.filterAttrs (
    _name: secret: activeOnHost secret && secret.backend == "sops"
  ) cfg.files;

  toSopsSecret =
    name: secret:
    {
      sopsFile =
        if secret.sopsFile == null then
          throw "finite.secrets.files.${name}: backend = \"sops\" requires sopsFile"
        else
          secret.sopsFile;
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
        backend = lib.mkOption {
          type = lib.types.enum [
            "legacy"
            "sops"
          ];
          default = "legacy";
          description = "Secret source backend for this migration entry.";
        };

        scope = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ hostName ];
          description = "Production hosts this secret entry is active on. Empty means every host.";
        };

        legacyPath = lib.mkOption {
          type = lib.types.str;
          example = "/etc/finite/metrics-remote-write.env";
          description = "Current operator-placed plaintext path used while backend is legacy.";
        };

        sopsFile = lib.mkOption {
          type = lib.types.nullOr lib.types.path;
          default = null;
          example = "../secrets/shared/metrics-remote-write.env";
          description = "Tracked encrypted SOPS source file. Required when backend is sops.";
        };

        destinationPath = lib.mkOption {
          type = lib.types.str;
          default = "/run/secrets/finite/${name}";
          description = "Runtime path consumers should read after this entry moves to SOPS.";
        };

        path = lib.mkOption {
          type = lib.types.str;
          readOnly = true;
          default = if config.backend == "legacy" then config.legacyPath else config.destinationPath;
          description = "Resolved runtime path for service modules to consume.";
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
    sops.secrets = lib.mapAttrs toSopsSecret sopsBackedFiles;

    assertions =
      lib.mapAttrsToList (name: secret: {
        assertion = secret.backend != "sops" || secret.sopsFile != null;
        message = "finite.secrets.files.${name}: backend = \"sops\" requires sopsFile";
      }) cfg.files
      ++ lib.mapAttrsToList (name: secret: {
        assertion = builtins.match "[0-7][0-7][0-7][0-7]" secret.mode != null;
        message = "finite.secrets.files.${name}: mode must be four octal digits";
      }) cfg.files;
  };
}
