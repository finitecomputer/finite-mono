# finite-saas-core — the control plane API (was k3s Deployment
# finite-system/finite-saas-core on old lat1). Binds 127.0.0.1:4200; Caddy
# routes finite.computer/internal/finite-private/* here (the limiter's usage
# API — the protected invariant of the cutover).
{ finitePackages, ... }:
{
  systemd.services.finite-saas-core = {
    description = "Finite SaaS core (control plane API)";
    wants = [ "network-online.target" ];
    after = [
      "network-online.target"
      "postgresql.service"
    ];
    requires = [ "postgresql.service" ];
    wantedBy = [ "multi-user.target" ];

    # Non-secret config, ported from the k8s manifest env + ConfigMap
    # finite-computer-config (infra/hosts/lat1/k8s/).
    environment = {
      FC_CORE_BIND = "127.0.0.1:4200";
      FC_CORE_RELAY_STATE_DIR = "/var/lib/finite-saas-core/relay";
      # Phase 1 ships parser/schema compatibility with first use disabled.
      # Flip only in a later config generation after that closure is live; see
      # the rollback-rescue procedure in infra/runbooks/runtime-image.md.
      FC_CORE_ENABLE_RUNTIME_UPGRADES = "false";
      # Public Stripe price id (ConfigMap value; not a secret).
      STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID = "price_1TouEuFwiFww4itkeTQNPYR6";
      # Verified emails allowed on /api/core/v1/admin/*; empty fails closed.
      FC_CORE_ADMIN_EMAILS = "paul@finite.vip,austin@finite.vip,skyler@finitesupply.xyz";
    };

    serviceConfig = {
      ExecStart = "${finitePackages.finite-saas-core}/bin/finite-saas-core";
      DynamicUser = true;
      StateDirectory = "finite-saas-core"; # relay state (was PVC finite-saas-core-relay-state)
      # Operator-created, root:root 0600. Variable NAMES only (values come
      # from k8s Secret finite-computer-secrets on old lat1 — see
      # infra/hosts/lat1/README.md secrets inventory):
      #   FC_CORE_DATABASE_URL  postgresql://finite:<POSTGRES_PASSWORD>@127.0.0.1:5432/finite_core
      #                         (the k8s manifest composed this from POSTGRES_PASSWORD)
      #   FC_CORE_API_TOKEN
      #   FC_FINITE_PRIVATE_USAGE_API_TOKEN  (pairs with the Tinfoil-sealed
      #                         FINITE_USAGE_API_SERVICE_KEY — do NOT rotate at cutover)
      EnvironmentFile = "/etc/finite/core.env";
      Restart = "on-failure";
      RestartSec = 2;
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      ProtectKernelTunables = true;
      ProtectControlGroups = true;
      RestrictSUIDSGID = true;
    };
  };
}
