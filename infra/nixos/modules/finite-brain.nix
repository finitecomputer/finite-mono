# finite-brain — first-party server on the canonical brain.finite.computer
# origin and behind the finite.computer dashboard's embedded client proxy. It
# binds loopback only. WorkOS protects the embedded Product Client; Brain owns
# route-level auth through signed Nostr request proofs.
{ config, finitePackages, ... }:
let
  mailEnvironmentFile = "/etc/finite-saas/sites.env";
in
{
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
