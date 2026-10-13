# finite-saas-dashboard — the Next.js dashboard (was k3s Deployment
# finite-dashboard, NodePort 30080, image built on-box). Now an OCI container
# from GHCR, digest-pinned, loopback-only; Caddy routes finite.computer -> it.
{ ... }:
{
  virtualisation.oci-containers.containers.finite-saas-dashboard = {
    # October 12 signup and fresh Agent recovery status, built from 036b2e58.
    # Core/Chat Rust retains qualified a3cbda97 inputs; excludes PR1055.
    # https://github.com/finitecomputer/finite-mono/actions/runs/38314673205
    image = "ghcr.io/finitecomputer/finite-saas-dashboard@sha256:ca15806f1a87dd5e3e4e945396e547053cc3223f16472853282dbb1f7a7ad815";

    # Host networking: the dashboard must reach core on the HOST loopback
    # (127.0.0.1:4200) and itself bind 127.0.0.1:3000 (HOSTNAME below). With
    # bridge networking neither side of that loopback contract holds.
    extraOptions = [ "--network=host" ];

    # Non-secret config mirrored from finite-computer-config
    # (infra/hosts/lat1/k8s/configmap.yaml), with FC_CORE_BASE_URL rewritten
    # from the k8s service name to the local core bind.
    environment = {
      HOSTNAME = "127.0.0.1"; # Next.js bind address (loopback-only)
      PORT = "3000";
      FC_WORKOS_AUTH_ENABLED = "true";
      FC_DASHBOARD_RUNTIME_MODE = "customer";
      FC_DASHBOARD_DEFAULT_RUNNER_CLASS = "kata";
      FC_DASHBOARD_RUNNER_CLASSES = "kata";
      # The Core request gate remains the independent authority; the lat1 Kata
      # Runner has a dedicated restricted retirement archive path.
      FC_DASHBOARD_ENABLE_RUNTIME_RETIREMENT = "true";
      FC_CORE_BASE_URL = "http://127.0.0.1:4200";
      FC_HOSTED_WEB_DEVICE_URL = "http://127.0.0.1:38918";
      FC_BRAIN_UPSTREAM_URL = "http://127.0.0.1:3015";
      FC_BRAIN_PUBLIC_ORIGIN = "https://brain.finite.computer";
      # One Sites registry owns viewing and publishing authorization.
      FC_SITES_UPSTREAM_URL = "https://finite.site";
      FC_CHAT_RELAY_TIMEOUT_MS = "30000";
      FC_DASHBOARD_BASE_URL = "https://finite.computer";
      NEXT_PUBLIC_WORKOS_REDIRECT_URI = "https://finite.computer/callback";
      STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID = "price_1TsqWWA50jhCdjMEhQLEBpvR";
      # Front door to the new Finite product (finite.chat). Off = today's
      # dashboard exactly. "true" sends signed-in accounts without a legacy
      # agent (or marked migrated in Core) to the target, /signup to the new
      # sign-up screen, and stops legacy agent creation; operators, legacy
      # agent owners, and anyone Core cannot be read for stay here. A target
      # on this dashboard's own origin is refused and keeps the switch off.
      # See finitecomputer-v2/apps/dashboard/README.md ("Front door").
      FC_FRONT_DOOR_ENABLED = "false";
      FC_FRONT_DOOR_TARGET_URL = "https://finite.chat/";
      FC_FRONT_DOOR_SIGNUP_URL = "https://finite.chat/auth/login";
    };

    # Operator-created, root:root 0600. Variable NAMES only (values from k8s
    # Secret finite-computer-secrets on old lat1 — dashboard manifest env):
    #   FC_CORE_API_TOKEN
    #   WORKOS_API_KEY
    #   WORKOS_CLIENT_ID
    #   WORKOS_COOKIE_PASSWORD
    #   FC_WORKOS_OPERATOR_ORG_ID        UI gate only; Core revalidates org_id
    #   STRIPE_SECRET_KEY
    #   STRIPE_WEBHOOK_SECRET
    #   GOOGLE_WORKSPACE_CLIENT_ID        (optional in the manifest)
    #   GOOGLE_WORKSPACE_CLIENT_SECRET    (optional in the manifest)
    #   FC_CORE_BRAIN_IDENTITY_URL        optional; Core's private Brain identity
    #   FC_CORE_BRAIN_OBSERVATION_TOKEN   listener, set both or neither (FIN-122)
    environmentFiles = [
      "/etc/finite/dashboard.env"
      "/etc/finite/hosted-web-device.env"
      "/etc/finite/sites-viewer-session.env"
    ];
  };
}
