# finite-brain — first-party server on the canonical brain.finite.computer
# origin and behind the finite.computer dashboard's embedded client proxy. It
# binds loopback only. WorkOS protects the embedded Product Client; Brain owns
# route-level auth through signed Nostr request proofs.
{
  config,
  finitePackages,
  pkgs,
  ...
}:
let
  mailEnvironmentFile = "/etc/finite-saas/sites.env";
  labelsPath = "/run/finite-brain-labels/principals.json";
  labelsSocket = "/run/finite-brain-labels/refresh.sock";
in
{
  users.groups.finite-brain-labels = { };
  users.users.finite_brain_labels = {
    isSystemUser = true;
    group = "finite-brain-labels";
  };
  systemd.tmpfiles.rules = [ "d /run/finite-brain-labels-root 0755 root root - -" ];
  # Existing local peer authentication covers this dedicated Unix user. Keep
  # role setup in its own unit so adding labels does not restart PostgreSQL.
  systemd.services.finite-brain-label-grants = {
    description = "Grant read-only public identity columns to Brain label observer";
    wantedBy = [ "postgresql.service" ];
    after = [
      "postgresql.service"
      "finite-saas-core.service"
    ];
    # Observe an already-running database; never undo an operator's DB stop.
    unitConfig.Requisite = "postgresql.service";
    partOf = [ "postgresql.service" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      User = "postgres";
      TimeoutStartSec = 15;
      ExecStart = pkgs.writeShellScript "grant-brain-label-reader" ''
        set -euo pipefail
        ${config.services.postgresql.package}/bin/psql --no-psqlrc --quiet --set=ON_ERROR_STOP=1 --dbname=finite_core <<'SQL'
        SET statement_timeout = '5s';
        SET lock_timeout = '1s';
        DO $role$
        BEGIN
          IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'finite_brain_labels') THEN
            CREATE ROLE finite_brain_labels LOGIN;
          END IF;
        END
        $role$;
        GRANT CONNECT ON DATABASE finite_core TO finite_brain_labels;
        GRANT USAGE ON SCHEMA public TO finite_brain_labels;
        GRANT SELECT (workos_user_id, normalized_email, link_status) ON users TO finite_brain_labels;
        GRANT SELECT (id, display_name) ON projects TO finite_brain_labels;
        GRANT SELECT (project_id, health_reporting_npub) ON agent_runtimes TO finite_brain_labels;
        SQL
      '';
    };
  };
  # Disposable display cache. Dedicated peer-authenticated Postgres role with
  # column SELECT grants; no credentials or Core API tokens. The worker's root
  # filesystem exposes only the read-only sources, Nix closure, socket, and output.
  systemd.services.finite-brain-labels = {
    description = "Refresh private Brain principal labels on demand";
    wantedBy = [ "multi-user.target" ];
    after = [ "finite-brain-label-grants.service" ];
    # Keep the socket alive through database stops. Future demand retries after
    # the explicitly restarted database has reestablished the reader grants.
    wants = [ "finite-brain-label-grants.service" ];
    environment = {
      FINITE_BRAIN_LABEL_DATABASE_URL = "host=/run/postgresql user=finite_brain_labels dbname=finite_core";
      FINITE_BRAIN_DB = "/sources/brain/finite-brain.sqlite3";
      FINITECHAT_HOSTED_DATA_ROOT = "/sources/hosted";
      FINITE_BRAIN_PRINCIPAL_LABELS = "/output/principals.json";
      FINITE_BRAIN_LABEL_SOCKET = "/output/refresh.sock";
    };
    serviceConfig = {
      Type = "simple";
      Restart = "on-failure";
      RestartSec = 30;
      ExecStart = "${finitePackages.finite-brain}/bin/finite-brain-labels";
      User = "finite_brain_labels";
      # Read DynamicUser-owned SQLite sources, confined to these explicit binds.
      CapabilityBoundingSet = [ "CAP_DAC_READ_SEARCH" ];
      AmbientCapabilities = [ "CAP_DAC_READ_SEARCH" ];
      RootDirectory = "/run/finite-brain-labels-root";
      BindReadOnlyPaths = [
        "/nix/store"
        "/run/postgresql"
        "/var/lib/private/finitebrain:/sources/brain"
        "/var/lib/private/finitechat-hosted-device/users:/sources/hosted/users"
      ];
      BindPaths = [ "/run/finite-brain-labels:/output" ];
      RestrictAddressFamilies = [ "AF_UNIX" ];
      Group = "finite-brain-labels";
      RuntimeDirectory = "finite-brain-labels";
      RuntimeDirectoryMode = "0750";
      RuntimeDirectoryPreserve = true;
      UMask = "0027";
      TimeoutStartSec = 35;
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      ReadWritePaths = [ "+/output" ];
    };
  };
  systemd.services.finite-brain-app = {
    description = "FiniteBrain Rust application server";
    wants = [ "network-online.target" ];
    # Brain no longer calls the Identity Directory or SaaS Core at request
    # time (auth-kernel cut): invitations are capability tokens and finite.vip
    # NIP-05 resolves through public internet fetch. No service requires are
    # left; boot ordering follows network-online only.
    after = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];

    environment = {
      FBRAIN_CONFIG_DIR = "/var/lib/finitebrain/fbrain";
      FINITE_BRAIN_ADDR = "127.0.0.1:3015";
      FINITE_BRAIN_DB = "/var/lib/finitebrain/finite-brain.sqlite3";
      FINITE_BRAIN_PUBLIC_BASE_URL = "https://brain.finite.computer";
      FINITE_BRAIN_SERVER_URL = "https://brain.finite.computer";
      FINITE_BRAIN_INVITE_MAILER = "resend";
      FINITE_BRAIN_INVITE_MAIL_FROM = "Finite Brain <brain@finite.chat>";
      FINITE_BRAIN_PRINCIPAL_LABELS = labelsPath;
      FINITE_BRAIN_LABEL_SOCKET = labelsSocket;
    };

    serviceConfig = {
      # systemd's default soft fd limit (1024) starved the hosted-device daemon
      # of sockets during the 2026-08-12 sync burst (reqwest Client::new EMFILE
      # -> "Chat is unavailable"). Raise it for every long-running platform
      # service; the hard limit already allows it.
      LimitNOFILE = 65536;
      ExecStart = "${finitePackages.finite-brain}/bin/finite-brain";
      EnvironmentFile = [
        # Existing send-only Resend credential shared with Sites and Identity.
        # Brain still owns its invitation content and access policy. The
        # retired identity-operator.env and brain-authority.env loads are gone:
        # the server no longer reads FINITE_IDENTITY_OPERATOR_TOKEN or
        # FC_CORE_API_TOKEN.
        mailEnvironmentFile
      ];
      DynamicUser = true;
      SupplementaryGroups = [ "finite-brain-labels" ];
      # SQLite restored from smoke at cutover; real path under DynamicUser:
      # /var/lib/private/finitebrain/finite-brain.sqlite3.
      StateDirectory = "finitebrain";
      WorkingDirectory = "/var/lib/finitebrain";
      Restart = "always";
      RestartSec = 3;
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectSystem = "full";
      ReadWritePaths = [ "/var/lib/finitebrain" ];
    };
  };

  assertions = [
    {
      assertion =
        config.systemd.services.finite-brain-app.environment.FINITE_BRAIN_INVITE_MAILER == "resend"
        &&
          config.systemd.services.finite-brain-app.environment.FINITE_BRAIN_INVITE_MAIL_FROM
          == "Finite Brain <brain@finite.chat>"
        && builtins.elem mailEnvironmentFile config.systemd.services.finite-brain-app.serviceConfig.EnvironmentFile;
      message = "production Finite Brain must use the shared Resend delivery credential";
    }
  ];
}
