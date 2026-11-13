# The FIN-463 mover on lat-2: it moves each finite.computer agent to
# finite.chat, as finite.chat's mailbox asks (finite-mover in
# finitecomputer-v2/crates). It reads Core's database through a read-only
# role, changes Core only through four exact subcommands of Core's own CLI,
# and reads each agent's tree on its Runner host over ssh, where
# finite-mover-host.nix forces the reader. Which brains each agent's
# person owns it reads from finite-brain's database, only through one
# fixed command run as finite-brain's own user (FIN-499).
#
# Two keys are made once, here on the host, and never leave it: the mover's
# Nostr key (`mover.key`, which finite.chat knows by its npub) and its ssh
# key for the Runner hosts. Neither is in the store, the repo or a log.
#
# The timers are off until `timers.enable`: deploy first, give finite.chat
# the npub, then turn them on. Delete this module with the rest of the move
# (FIN-463's removal plan), with the role and both keys.
{
  config,
  lib,
  pkgs,
  finitePackages,
  ...
}:
let
  cfg = config.finite.mover;
  core = config.systemd.services.finite-saas-core;

  stateDir = "/var/lib/finite-mover";
  keyFile = "${stateDir}/mover.key";
  sshKey = "${stateDir}/ssh/id_ed25519";
  configFile = "/etc/finite-mover/config.json";
  knownHosts = "/etc/finite-mover/known_hosts";

  # The only Core subcommands the mover may run (finite-mover's core_cli.rs).
  subcommands = [
    "runtime-stop-exact"
    "runtime-retire-exact"
    "runtime-mark-moved-exact"
    "runtime-restart-exact"
  ];
  # finite-mover stops a Core call at its wait plus CORE_MARGIN (300 s), but
  # can't signal a root process; the transient unit ends there by itself.
  coreDeadline = cfg.core.waitSeconds + 300;

  # Core's own environment: its unit's variables and its root-only
  # EnvironmentFile, which systemd reads; this script never sees a secret.
  # Core runs as a DynamicUser, so this runs as that same user
  # (finite-saas-core), in a transient unit. Arguments are passed as they
  # are: no $VARIABLE in them is expanded.
  coreEnvironment = lib.filterAttrs (name: value: value != null && name != "PATH") core.environment;
  coreWrapper = pkgs.writeShellScript "finite-mover-core" ''
    set -eu
    case "''${1-}" in
      ${lib.concatStringsSep "|" subcommands}) ;;
      *)
        echo "finite-mover-core: only ${lib.concatStringsSep ", " subcommands}" >&2
        exit 64
        ;;
    esac
    exec ${config.systemd.package}/bin/systemd-run \
      --quiet --wait --pipe --collect \
      --service-type=exec \
      --expand-environment=no \
      --description="finite-saas-core $1 for the finite.chat mover" \
      --property=DynamicUser=yes \
      --property=User=finite-saas-core \
      --property=EnvironmentFile=${core.serviceConfig.EnvironmentFile} \
      --property=RuntimeMaxSec=${toString coreDeadline} \
      ${
        lib.concatStringsSep " " (
          lib.mapAttrsToList (
            name: value: lib.escapeShellArg "--setenv=${name}=${toString value}"
          ) coreEnvironment
        )
      } \
      -- ${cfg.corePackage}/bin/finite-saas-core "$@"
  '';

  # finite-brain's database, read for the mover by `finite-mover-brains`
  # as the database's own owner (finite-brain's dynamic user), never as the
  # mover: a file of the database another user makes (its `-shm`) is one
  # finite-brain can't open (finite-litestream.nix). The owner is read off
  # the database each time (a dynamic uid isn't fixed, and a transient
  # DynamicUser unit doesn't share it); a root-owned database is refused.
  # Its folder is bound in by systemd, which alone passes
  # /var/lib/private; the reader opens it read-only, refuses one another
  # user owns, and has no network.
  brainMount = "/run/finite-mover-brain";
  brainsWrapper = pkgs.writeShellScript "finite-mover-brains" ''
    set -eu
    if [ "$#" -ne 0 ]; then
      echo "finite-mover-brains: takes no arguments" >&2
      exit 64
    fi
    database=${lib.escapeShellArg cfg.brain.database}
    owner=$(${pkgs.coreutils}/bin/stat -c %u "$database")
    group=$(${pkgs.coreutils}/bin/stat -c %g "$database")
    if [ "$owner" = 0 ] || [ "$group" = 0 ]; then
      echo "finite-mover-brains: $database is root's; refusing" >&2
      exit 1
    fi
    exec ${config.systemd.package}/bin/systemd-run \
      --quiet --wait --pipe --collect \
      --service-type=exec \
      --description="finite-brain's owned brains, read for the finite.chat mover" \
      --uid="$owner" --gid="$group" \
      --property=BindPaths=${dirOf cfg.brain.database}:${brainMount} \
      --property=PrivateNetwork=yes \
      --property=PrivateTmp=yes \
      --property=ProtectSystem=strict \
      --property=ProtectHome=yes \
      --property=NoNewPrivileges=yes \
      --property=RuntimeMaxSec=120 \
      -- ${cfg.package}/bin/finite-mover-brains ${brainMount}/${baseNameOf cfg.brain.database}
  '';

  ssh = [
    "${pkgs.openssh}/bin/ssh"
    "-i"
    sshKey
    "-o"
    "BatchMode=yes"
    "-o"
    "IdentitiesOnly=yes"
    "-o"
    "StrictHostKeyChecking=yes"
    "-o"
    "UserKnownHostsFile=${knownHosts}"
    "-o"
    "GlobalKnownHostsFile=/dev/null"
  ];

  moverConfig = {
    finite_chat = cfg.finiteChat;
    key_file = keyFile;
    locks = "${stateDir}/locks";
    fleet.postgres = {
      conninfo = "host=/run/postgresql user=finite_mover dbname=${cfg.postgres.database}";
      sizes_file = "${stateDir}/sizes.json";
      brains = [
        "${config.security.wrapperDir}/sudo"
        "-n"
        "${brainsWrapper}"
      ];
    };
    core = {
      inherit (cfg.core) program;
      admin_email = cfg.core.adminEmail;
      admin_workos_user_id = cfg.core.adminWorkosUserId;
      wait_seconds = cfg.core.waitSeconds;
    };
    hosts = lib.mapAttrs (_: host: ssh ++ [ "finite-mover@${host.address}" ]) cfg.hosts;
  };

  roleSql = "${cfg.package}/share/finite-mover/core-role.sql";

  # Made once, as finite-mover, and never replaced: a key that exists is
  # kept whatever it holds (the mover refuses one that's malformed or
  # readable by others). Nothing secret is printed.
  makeKeys = pkgs.writeShellScript "finite-mover-keys" ''
    set -eu
    umask 077
    cd ${stateDir}
    if [ ! -e mover.key ]; then
      new=$(mktemp mover.key.XXXXXX)
      od -An -v -tx1 -N32 /dev/urandom | tr -d ' \n' > "$new"
      # ln never replaces an existing name.
      ln "$new" mover.key
      rm "$new"
      echo "made the mover key"
    fi
    if [ ! -e ssh/id_ed25519 ]; then
      rm -rf ssh/new
      mkdir ssh/new
      ssh-keygen -q -t ed25519 -N "" -C "finite-mover@${config.networking.hostName}" -f ssh/new/id_ed25519
      mv -f ssh/new/id_ed25519.pub ssh/id_ed25519.pub
      ln ssh/new/id_ed25519 ssh/id_ed25519
      rm -rf ssh/new
      echo "made the mover's ssh key"
    fi
    if [ ! -e ssh/id_ed25519.pub ]; then
      ssh-keygen -y -f ssh/id_ed25519 > ssh/id_ed25519.pub
    fi
    echo "the mover's ssh key, for finite.moverHost.authorizedKeys: $(cat ssh/id_ed25519.pub)"
  '';

  run = operation: {
    description = "finite-mover ${operation}";
    requires = [
      "finite-mover-keys.service"
      "finite-mover-postgres.service"
    ];
    after = [
      "finite-mover-keys.service"
      "finite-mover-postgres.service"
      "network-online.target"
    ];
    wants = [ "network-online.target" ];
    restartTriggers = [ config.environment.etc."finite-mover/config.json".source ];
    serviceConfig = {
      Type = "oneshot";
      User = "finite-mover";
      Group = "finite-mover";
      ExecStart = "${cfg.package}/bin/finite-mover --config ${configFile} ${operation}";
      # No NoNewPrivileges: Core's subcommands go through sudo.
      ProtectSystem = "strict";
      ReadWritePaths = [ stateDir ];
      ProtectHome = true;
      PrivateTmp = true;
    };
  };

  timer = calendar: extra: {
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnCalendar = calendar;
    }
    // extra;
  };
in
{
  options.finite.mover = {
    enable = lib.mkEnableOption "the FIN-463 mover of finite.computer agents to finite.chat";
    timers.enable = lib.mkEnableOption ''
      the mover's timers: mailbox every 10 s, roster every 10 min, sizes
      nightly. Only once finite.chat's `migrations.mover` names this
      mover's npub'';

    package = lib.mkOption {
      type = lib.types.package;
      default = finitePackages.finite-mover;
      defaultText = lib.literalExpression "finitePackages.finite-mover";
      description = "The mover, from the same release as Core.";
    };
    corePackage = lib.mkOption {
      type = lib.types.package;
      default = finitePackages.finite-saas-core;
      defaultText = lib.literalExpression "finitePackages.finite-saas-core";
      description = "Core, whose CLI the mover's four subcommands run.";
    };
    finiteChat = lib.mkOption {
      type = lib.types.str;
      default = "https://finite.chat";
      description = "finite.chat's origin, whose mailbox the mover works.";
    };
    postgres.database = lib.mkOption {
      type = lib.types.str;
      default = "finite_core";
      description = "Core's database, which the finite_mover role reads.";
    };
    core = {
      adminEmail = lib.mkOption {
        type = lib.types.str;
        description = "The operator identity Core records as acting for the mover (an audit fact, not a secret). Core links it if it's missing.";
      };
      adminWorkosUserId = lib.mkOption {
        type = lib.types.str;
        description = "That operator identity's WorkOS user id.";
      };
      waitSeconds = lib.mkOption {
        type = lib.types.ints.positive;
        default = 900;
        description = "How long Core is waited on for a stop, retirement or restart.";
      };
      program = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [
          "${config.security.wrapperDir}/sudo"
          "-n"
          "${coreWrapper}"
        ];
        defaultText = lib.literalExpression ''[ "/run/wrappers/bin/sudo" "-n" "<finite-mover-core>" ]'';
        description = "The command that runs Core's CLI; the subcommand follows.";
      };
    };
    brain = {
      database = lib.mkOption {
        type = lib.types.str;
        default = "/var/lib/private/finitebrain/finite-brain.sqlite3";
        description = "finite-brain's database, at its real path under /var/lib/private (finite-brain.nix's FINITE_BRAIN_DB). Its owner reads it for the mover.";
      };
    };
    hosts = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.submodule {
          options = {
            address = lib.mkOption {
              type = lib.types.str;
              example = "10.254.3.2";
              description = "Where the Runner host's sshd answers.";
            };
            hostKey = lib.mkOption {
              type = lib.types.str;
              example = "ssh-ed25519 AAAA...";
              description = "The Runner host's ssh host key, as /etc/ssh/ssh_host_ed25519_key.pub has it.";
            };
          };
        }
      );
      default = { };
      example = lib.literalExpression ''
        {
          finite-lat-3 = { address = "10.254.3.2"; hostKey = "ssh-ed25519 AAAA..."; };
        }
      '';
      description = "Each Runner host by Core's source_host_id, reached as finite-mover@address, its host key pinned.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = config.services.postgresql.enable;
        message = "finite.mover reads Core's database by peer authentication: Postgres must be local";
      }
      {
        assertion = lib.all (
          host: builtins.match "[a-z0-9@.-]+ [A-Za-z0-9+/=]+( .*)?" host.hostKey != null
        ) (lib.attrValues cfg.hosts);
        message = "finite.mover.hosts.<id>.hostKey must be one ssh public key";
      }
    ];

    users.groups.finite-mover = { };
    users.users.finite-mover = {
      isSystemUser = true;
      group = "finite-mover";
    };

    systemd.tmpfiles.rules = [
      "d ${stateDir} 0700 finite-mover finite-mover -"
      "d ${stateDir}/locks 0700 finite-mover finite-mover -"
      "d ${stateDir}/ssh 0700 finite-mover finite-mover -"
    ];

    environment.systemPackages = [ cfg.package ];
    environment.etc."finite-mover/config.json" = {
      text = builtins.toJSON moverConfig;
      mode = "0640";
      user = "root";
      group = "finite-mover";
    };
    environment.etc."finite-mover/known_hosts".text = lib.concatStrings (
      lib.mapAttrsToList (_: host: "${host.address} ${host.hostKey}\n") cfg.hosts
    );

    systemd.services.finite-mover-keys = {
      description = "Make the finite.chat mover's keys, once";
      wantedBy = [ "multi-user.target" ];
      after = [ "systemd-tmpfiles-setup.service" ];
      path = [
        pkgs.coreutils
        pkgs.openssh
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        User = "finite-mover";
        Group = "finite-mover";
        ExecStart = makeKeys;
        UMask = "0077";
        ProtectSystem = "strict";
        ReadWritePaths = [ stateDir ];
        ProtectHome = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
      };
    };

    # The role reads exactly core-role.sql's columns, from the mover's own
    # release. Postgres applies it: a superuser's GRANT is recorded as the
    # table owner's (Core's database owner), and never silently does
    # nothing. It needs Core's tables, so it follows Core; a grant dropped
    # from the file later is not revoked here.
    services.postgresql = {
      ensureUsers = [ { name = "finite_mover"; } ];
      identMap = ''
        finite-mover finite-mover finite_mover
      '';
      authentication = ''
        local ${cfg.postgres.database} finite_mover peer map=finite-mover
      '';
    };
    systemd.services.finite-mover-postgres = {
      description = "Grant the finite.chat mover its read-only columns of Core's database";
      wantedBy = [ "multi-user.target" ];
      requires = [ "postgresql.target" ];
      after = [
        "postgresql.target"
        "finite-saas-core.service"
      ];
      restartTriggers = [ roleSql ];
      path = [ config.services.postgresql.finalPackage ];
      environment.PGPORT = toString config.services.postgresql.settings.port;
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        User = "postgres";
        Group = "postgres";
        ExecStart = "${config.services.postgresql.finalPackage}/bin/psql --dbname=${cfg.postgres.database} --set=ON_ERROR_STOP=1 --single-transaction --file=${roleSql}";
      };
    };

    security.sudo.extraRules = [
      {
        users = [ "finite-mover" ];
        runAs = "root";
        commands =
          map (subcommand: {
            command = "${coreWrapper} ${subcommand} *";
            options = [ "NOPASSWD" ];
          }) subcommands
          # `""`: no arguments at all.
          ++ [
            {
              command = ''${brainsWrapper} ""'';
              options = [ "NOPASSWD" ];
            }
          ];
      }
    ];

    systemd.services.finite-mover-mailbox = run "mailbox";
    systemd.services.finite-mover-roster = run "roster";
    systemd.services.finite-mover-sizes = run "sizes";

    systemd.timers = lib.mkIf cfg.timers.enable {
      # A run still going when the next is due is simply not started again;
      # the mailbox's own lock covers a hand-started run.
      finite-mover-mailbox = timer "*:*:0/10" { AccuracySec = "1s"; };
      finite-mover-roster = timer "*:0/10" { };
      # Every tree is walked (metadata only): at night, spread out.
      finite-mover-sizes = timer "*-*-* 02:00:00" { RandomizedDelaySec = "3h"; };
    };
  };
}
