# finitesitesd — Finite Sites registry, publishing API, Git smart HTTP, and
# site serving for *.finite.chat / *.docs.finite.chat on 127.0.0.1:8787.
# Data: /var/lib/finite-sites (restored from lat2 at cutover and included in
# the coordinated v3 Recovery Set).
{
  config,
  finitePackages,
  kataPackages,
  pkgs,
  ...
}:
let
  # Agent Runtimes share this host but use the QEMU Kata profile patched to
  # 4 vCPU / 8 GiB. Stateful App Outputs select the kata-clh shim alias and
  # therefore this independent small-guest profile. Do not collapse the two:
  # doing so makes every tiny app consume an Agent-sized VM envelope.
  appKataConfiguration = pkgs.runCommand "kata-configuration-clh-finite-app.toml" { } ''
    sed -e 's/^default_vcpus = .*/default_vcpus = 1/' \
        -e 's/^default_memory = .*/default_memory = 512/' \
        ${kataPackages.kata-runtime}/share/defaults/kata-containers/configuration-clh.toml \
        > "$out"
    grep -q '^default_vcpus = 1$' "$out"
    grep -q '^default_memory = 512$' "$out"
  '';
in
{
  users.users.finite-sites = {
    isSystemUser = true;
    group = "finite-sites";
  };
  users.groups.finite-sites = { };

  assertions = [
    {
      assertion = config.virtualisation.containerd.enable;
      message = "Finite Sites Kata apps require the shared containerd host runtime";
    }
    {
      assertion = config.security.sudo.enable;
      message = "Finite Sites Kata apps require the narrow nerdctl sudo rule";
    }
  ];

  # containerd discovers containerd-shim-kata-clh-v2 from the Kata package
  # already placed on its PATH by finite-saas-runner.nix. The shim alias reads
  # this VMM-specific file instead of the Agent Runtime configuration.toml.
  environment.etc."kata-containers/configuration-clh.toml".source = appKataConfiguration;

  # Keep the daemon unprivileged. The one root path is the exact immutable
  # nerdctl wrapper passed in ExecStart; all argv is daemon-constructed and
  # tenant code remains one shell argument inside its Kata guest.
  security.sudo.extraRules = [
    {
      users = [ "finite-sites" ];
      runAs = "root";
      commands = [
        {
          command = "${kataPackages.nerdctl}/bin/nerdctl";
          options = [ "NOPASSWD" ];
        }
      ];
    }
  ];

  systemd.services.finite-saas-sites = {
    description = "Finite Sites (registry, publishing API, site serving)";
    wants = [ "network-online.target" ];
    requires = [ "containerd.service" ];
    after = [
      "network-online.target"
      "containerd.service"
    ];
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
      ExecStart = ''
        ${finitePackages.finitesitesd}/bin/finitesitesd serve \
          --data /var/lib/finite-sites \
          --listen 127.0.0.1:8787 \
          --base-domain finite.chat \
          --document-base-domain docs.finite.chat \
          --api-url https://api.finite.chat \
          --git-url https://git.finite.chat \
          --site-scheme https \
          --site-port none \
          --mailer resend \
          --mail-from "Finite Sites <links@finite.chat>" \
          --app-runner kata \
          --app-sudo-path /run/wrappers/bin/sudo \
          --app-nerdctl-path ${kataPackages.nerdctl}/bin/nerdctl \
          --app-cni-path ${kataPackages.cni-plugins}/bin
      '';
      # Operator-created, root:root 0600. systemd reads EnvironmentFile before
      # starting the service under the finite-sites account.
      # Variable NAMES only (values from lat2's /etc/finite-saas/sites.env):
      #   RESEND_API_KEY
      # FINITE_IDENTITY_AUTHORITY is non-secret and supplied declaratively by
      # finite-identity.nix on the consolidated production host.
      # FINITE_SITES_VIEWER_SESSION_TOKEN in the second file must be exactly
      # 64 lowercase hex characters (`openssl rand -hex 32`).
      EnvironmentFile = [
        "/etc/finite-saas/sites.env"
        "/etc/finite/sites-viewer-session.env"
      ];
      Restart = "on-failure";
      RestartSec = 2;
      StateDirectory = "finite-sites";
      # sudo nerdctl must reach rootful containerd and CNI. Tenant isolation
      # is the Kata microVM boundary; technical-debt ledger item 10 records
      # this deliberate daemon-side hardening tradeoff and its delete condition.
      ProtectSystem = false;
      ReadWritePaths = [ ];
      ProtectHome = false;
      PrivateTmp = false;
      NoNewPrivileges = false;
      ProtectKernelTunables = true;
      ProtectControlGroups = true;
      RestrictSUIDSGID = true;
    };
  };
}
