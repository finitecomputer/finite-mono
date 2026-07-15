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
      # Parser/schema compatibility has been live since 2026-07-10. The
      # required active-operation preflight was clean before first use; see
      # the rollback-rescue procedure in infra/runbooks/runtime-image.md.
      FC_CORE_ENABLE_RUNTIME_UPGRADES = "true";
      # Core persists these bounded, public service endpoints into each new
      # RuntimeSpec. Runner keeps its process-global copy only for N-1 rows
      # without a spec during the expand window.
      FC_CORE_RUNTIME_ENV_JSON = builtins.toJSON {
        FINITE_SITES_API = "https://api.finite.chat";
        FINITE_BRAIN_SERVER_URL = "https://brain.finite.computer";
        FINITE_BRAIN_PUBLIC_BASE_URL = "https://brain.finite.computer";
      };
      # Public Stripe price id (ConfigMap value; not a secret).
      STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID = "price_1TsqWWA50jhCdjMEhQLEBpvR";
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
      #   FC_CORE_RUNNER_API_TOKEN       Runner-only lifecycle work capability
      #   FC_FINITE_PRIVATE_USAGE_API_TOKEN  (pairs with the Tinfoil-sealed
      #                         FINITE_USAGE_API_SERVICE_KEY — do NOT rotate at cutover)
      #   WORKOS_API_KEY                 Read-only user lookup after JWT validation
      #   WORKOS_CLIENT_ID               Expected AuthKit client_id/JWKS selector
      #   FC_WORKOS_OPERATOR_ORG_ID      Exact org_id required by admin routes
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
