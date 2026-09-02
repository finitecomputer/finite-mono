# finitesitesd — Finite Sites registry, publishing API, Git smart HTTP, and
# static site serving.
#
# App-plane hosts intentionally default to the content-pinned legacy daemon so
# routine lat2 deploys do not become the ADR 0028 cutover. The dedicated v2
# validation host opts into the in-tree static-only daemon explicitly.
{
  config,
  finitePackages,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.finite.sites;
in
{
  options.finite.sites = {
    mode = lib.mkOption {
      type = lib.types.enum [
        "legacy-canonical"
        "static-v2"
      ];
      default = "legacy-canonical";
      description = "Sites deployment contract for this host.";
    };
    package = lib.mkOption {
      type = lib.types.package;
      default = finitePackages.finitesitesd-legacy-canonical;
      defaultText = "finitePackages.finitesitesd-legacy-canonical";
      description = "finitesitesd package to run.";
    };
    dataDir = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/finite-sites";
      description = "Finite Sites registry, blob, and bare repository state directory.";
    };
    listen = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:8787";
      description = "Loopback listener for finitesitesd.";
    };
    baseDomain = lib.mkOption {
      type = lib.types.str;
      default = "finite.chat";
      description = "Wildcard site-serving base domain.";
    };
    apiUrl = lib.mkOption {
      type = lib.types.str;
      default = "https://api.finite.chat";
      description = "Public Sites API origin returned to clients.";
    };
    gitUrl = lib.mkOption {
      type = lib.types.str;
      default = "https://git.finite.chat";
      description = "Public Git smart-HTTP origin returned to clients.";
    };
    documentBaseDomain = lib.mkOption {
      type = lib.types.str;
      default = "docs.finite.chat";
      description = "Legacy document output domain; ignored by static-v2 mode.";
    };
    siteScheme = lib.mkOption {
      type = lib.types.str;
      default = "https";
      description = "Scheme used when constructing served site URLs.";
    };
    sitePort = lib.mkOption {
      type = lib.types.str;
      default = "none";
      description = "Served site URL port, or none when an edge proxy owns public ports.";
    };
    mailFrom = lib.mkOption {
      type = lib.types.str;
      default = "Finite Sites <links@finite.chat>";
      description = "Sender address passed to the configured Sites mailer.";
    };
  };

  config = {
    assertions = [
      {
        assertion =
          cfg.mode != "static-v2"
          || (
            cfg.baseDomain == "v2.finite.chat"
            && cfg.apiUrl == "https://v2.finite.chat"
            && cfg.gitUrl == "https://v2.finite.chat"
          );
        message = "static-v2 Sites mode is only for the dedicated v2.finite.chat validation host before canonical cutover";
      }
    ];

    users.users.finite-sites = {
      isSystemUser = true;
      group = "finite-sites";
    };
    users.groups.finite-sites = { };

    systemd.services.finite-saas-sites = {
      description = "Finite Sites (registry, publishing API, site serving)";
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];

      # finitesitesd deliberately delegates bare-repository setup and smart
      # HTTP to the Git executable. Nix systemd units do not inherit an
      # interactive shell PATH, so this runtime dependency must be explicit.
      path = [ pkgs.git ];

      serviceConfig = {
        # systemd's default soft fd limit (1024) starved the hosted-device daemon
        # of sockets during the 2026-08-12 sync burst (reqwest Client::new EMFILE
        # -> "Chat is unavailable"). Raise it for every long-running platform
        # service; the hard limit already allows it.
        LimitNOFILE = 65536;
        User = "finite-sites";
        Group = "finite-sites";
        # `--mailer` is required; omitting it is an error, not an implicit DevMailer.
        ExecStart =
          if cfg.mode == "legacy-canonical" then
            ''
              ${cfg.package}/bin/finitesitesd serve \
                --data ${cfg.dataDir} \
                --listen ${cfg.listen} \
                --base-domain ${cfg.baseDomain} \
                --document-base-domain ${cfg.documentBaseDomain} \
                --api-url ${cfg.apiUrl} \
                --git-url ${cfg.gitUrl} \
                --site-scheme ${cfg.siteScheme} \
                --site-port ${cfg.sitePort} \
                --mailer resend \
                --mail-from "${cfg.mailFrom}"
            ''
          else
            ''
              ${cfg.package}/bin/finitesitesd serve \
                --data ${cfg.dataDir} \
                --listen ${cfg.listen} \
                --base-domain ${cfg.baseDomain} \
                --api-url ${cfg.apiUrl} \
                --git-url ${cfg.gitUrl} \
                --site-scheme ${cfg.siteScheme} \
                --site-port ${cfg.sitePort} \
                --mailer resend \
                --mail-from "${cfg.mailFrom}"
            '';
        # Operator-created, root:root 0600. systemd reads EnvironmentFile before
        # starting the service under the finite-sites account.
        # Variable NAMES only; values are operator-created on each host:
        #   RESEND_API_KEY
        # FINITE_SITES_VIEWER_SESSION_TOKEN in the second file must be exactly
        # 64 lowercase hex characters (`openssl rand -hex 32`).
        EnvironmentFile = [
          "/etc/finite-saas/sites.env"
          "/etc/finite/sites-viewer-session.env"
        ];
        Restart = "on-failure";
        RestartSec = 2;
        StateDirectory = "finite-sites";
        ProtectSystem = "strict";
        ReadWritePaths = [ cfg.dataDir ];
        ProtectHome = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
        ProtectKernelTunables = true;
        ProtectControlGroups = true;
        RestrictSUIDSGID = true;
      };
    };
  };
}
