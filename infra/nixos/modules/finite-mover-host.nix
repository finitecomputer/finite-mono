# The FIN-463 mover's read-only access to Agent Runtime durable trees on a
# Kata Runner host. One system account, finite-mover, whose only login is
# the mover's ssh key, forced to run finite-mover-host for one
# kata/<durable_state_id>: `sizes`, `stream`, `secrets` or
# `owner-telegram-notice`, nothing else.
#
# The reader needs to read trees owned by the runtimes, so its wrapper holds
# CAP_DAC_READ_SEARCH (read and traverse only; no write, no other root power)
# and only the finite-mover group may run it. The crate never writes a tree,
# follows no link, and refuses to stage a running agent. Its one write is
# the owner notices' log, in a directory only finite-mover owns; it gives
# up the capability before the notice's curl runs.
#
# Nothing exists until authorizedKeys is set: an empty list (the default)
# changes no host. Installing the mover's public key is an operator step for
# Paul. Delete this module with the rest of the move (FIN-463's removal
# plan): the mover, the host reader, the DB role and this account.
{
  config,
  lib,
  finitePackages,
  ...
}:
let
  cfg = config.finite.moverHost;
  reader = "${config.security.wrapperDir}/finite-mover-host";
  # Each agent's owner notices: id, time and result, never the text.
  noticeLog = "/var/lib/finite-mover-host/notices";
  # restrict: no pty, no forwarding of any kind. The operation arrives in
  # SSH_ORIGINAL_COMMAND and is parsed by the reader, never by a shell.
  forced =
    key:
    ''restrict,command="${reader} --work-root ${cfg.workRoot} --notice-log ${noticeLog} --ssh" ${key}'';
in
{
  options.finite.moverHost = {
    authorizedKeys = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "ssh-ed25519 AAAA... finite-mover@finite-lat-2" ];
      description = "Public ssh keys of the move's mover on lat-2. Each may only run finite-mover-host; empty creates nothing.";
    };
    workRoot = lib.mkOption {
      type = lib.types.str;
      default = config.finite.kataRunnerHost.workRoot;
      description = "The Runner work root whose kata/<durable_state_id> trees the reader may read.";
    };
  };

  config = lib.mkIf (cfg.authorizedKeys != [ ]) {
    assertions = [
      {
        assertion = lib.hasPrefix "/" cfg.workRoot && !(lib.hasInfix " " cfg.workRoot);
        message = "finite.moverHost.workRoot must be an absolute path without spaces";
      }
    ];

    users.groups.finite-mover = { };
    users.users.finite-mover = {
      isSystemUser = true;
      group = "finite-mover";
      # sshd runs a forced command through the login shell.
      useDefaultShell = true;
      openssh.authorizedKeys.keys = map forced cfg.authorizedKeys;
    };

    systemd.tmpfiles.rules = [
      "d /var/lib/finite-mover-host 0700 finite-mover finite-mover -"
      "d ${noticeLog} 0700 finite-mover finite-mover -"
    ];

    security.wrappers.finite-mover-host = {
      source = "${finitePackages.finite-mover-host}/bin/finite-mover-host";
      owner = "root";
      group = "finite-mover";
      permissions = "u+rx,g+x";
      # Reads any agent's tree; and, its one exception to reading only,
      # seals a carried SimpleX: renames `agent/simplex` aside and rewrites
      # `config.yaml` with its owner kept (finite-mover-host src/simplex.rs).
      capabilities = "cap_dac_read_search,cap_dac_override,cap_chown+ep";
    };
  };
}
