# finite-gated — the Finite Auth Gate on 127.0.0.1:8792 behind
# auth.finite.computer. Viewers gate here: a browser hitting a non-public
# Finite Site output without a session is redirected (top-level) to this
# daemon; the human authenticates via WorkOS AuthKit; the gate returns with a
# short-lived, origin-bound, single-use vouch that finitesitesd verifies
# offline against the pinned gate public key. The gate never sets the site's
# viewer cookie. Stateless: no data directory.
#
# EnvironmentFile is operator-created, root:root 0600. Variable NAMES only
# (values live on the host, never in git):
#   FINITE_GATE_SIGNING_KEY       64 lowercase hex (openssl rand -hex 32);
#                                 its x-only public key is what finitesitesd
#                                 pins as FINITE_SITES_AUTH_GATE_PUBKEY
#                                 (`finite-gated` logs the pubkey at startup)
#   FINITE_GATE_WORKOS_CLIENT_ID  production WorkOS AuthKit client
#   FINITE_GATE_WORKOS_API_KEY    must be set together with the client id
#   FINITE_GATE_DEV_MODE          never set on a prod host: 1 = explicit
#                                 local-dev mode (fixed dev identity). With
#                                 neither WorkOS nor this flag the gate
#                                 refuses to start (fail closed).
{
  config,
  finitePackages,
  ...
}:
{
  users.users.finite-gate = {
    isSystemUser = true;
    group = "finite-gate";
  };
  users.groups.finite-gate = { };

  systemd.services.finite-gate = {
    description = "Finite Auth Gate (viewer authentication, vouch minting)";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];

    serviceConfig = {
      # See finitesitesd.nix for the fd-limit rationale (2026-08-12 burst).
      LimitNOFILE = 65536;
      User = "finite-gate";
      Group = "finite-gate";
      ExecStart = "${finitePackages.finite-gated}/bin/finite-gated";
      Environment = [
        "FINITE_GATE_LISTEN=127.0.0.1:8792"
        "FINITE_GATE_PUBLIC_URL=https://auth.finite.computer"
      ];
      # Operator-created, root:root 0600.
      EnvironmentFile = [ "/etc/finite-saas/gate.env" ];
      Restart = "on-failure";
      RestartSec = 2;
      NoNewPrivileges = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      PrivateTmp = true;
      ProtectKernelTunables = true;
      ProtectControlGroups = true;
      RestrictAddressFamilies = [
        "AF_INET"
        "AF_INET6"
      ];
    };
  };
}
