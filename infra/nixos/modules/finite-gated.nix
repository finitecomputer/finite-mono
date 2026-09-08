# Stateless gate: the account boundary verifies email; this daemon signs one
# origin-bound proof; Sites alone checks permissions. No second WorkOS session.
{ config, finitePackages, ... }:
{
  users.users.finite-gate = {
    isSystemUser = true;
    group = "finite-gate";
  };
  users.groups.finite-gate = { };
  systemd.services.finite-gate = {
    description = "Finite Auth Gate";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      User = "finite-gate";
      Group = "finite-gate";
      ExecStart = "${finitePackages.finite-gated}/bin/finite-gated";
      Environment = [
        "FINITE_GATE_LISTEN=127.0.0.1:8792"
        "FINITE_GATE_PUBLIC_URL=https://auth.${config.finite.sites.baseDomain}"
        "FINITE_GATE_SITE_BASE_DOMAIN=${config.finite.sites.baseDomain}"
        "FINITE_GATE_ACCOUNT_URL=https://finite.computer/site-auth"
      ];
      # Operator-owned 0600: FINITE_GATE_SIGNING_KEY and
      # FINITE_GATE_ACCOUNT_TOKEN (shared only with the account server).
      EnvironmentFile = [ "/etc/finite-saas/gate.env" ];
      Restart = "on-failure";
      RestartSec = 2;
      LimitNOFILE = 65536;
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
