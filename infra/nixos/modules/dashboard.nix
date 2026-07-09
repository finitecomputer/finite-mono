# finite-saas-dashboard — the Next.js dashboard (was k3s Deployment
# finite-dashboard, NodePort 30080, image built on-box). Now an OCI container
# from GHCR, digest-pinned, loopback-only; Caddy routes finite.computer -> it.
{ ... }:
{
  virtualisation.oci-containers.containers.finite-saas-dashboard = {
    # ########################################################################
    # ## TODO: pin the image digest after the first CI build pushes         ##
    # ## ghcr.io/finitecomputer/finite-saas-dashboard (built from           ##
    # ## infra/images/dashboard.Dockerfile). This placeholder digest WILL   ##
    # ## NOT PULL — the deploy fails loudly until it is replaced.           ##
    # ########################################################################
    image = "ghcr.io/finitecomputer/finite-saas-dashboard@sha256:TODO-pin-after-first-ci-build";

    # Host networking: the dashboard must reach core on the HOST loopback
    # (127.0.0.1:4200) and itself bind 127.0.0.1:3000 (HOSTNAME below). With
    # bridge networking neither side of that loopback contract holds.
    extraOptions = [ "--network=host" ];

    # Non-secret config: the 8 ConfigMap finite-computer-config keys
    # (infra/hosts/lat1/k8s/configmap.yaml), with FC_CORE_BASE_URL rewritten
    # from the k8s service name to the local core bind.
    environment = {
      HOSTNAME = "127.0.0.1"; # Next.js bind address (loopback-only)
      PORT = "3000";
      FC_WORKOS_AUTH_ENABLED = "true";
      FC_DASHBOARD_RUNTIME_MODE = "canary";
      FC_CORE_BASE_URL = "http://127.0.0.1:4200";
      FC_CHAT_RELAY_TIMEOUT_MS = "30000";
      FC_DASHBOARD_BASE_URL = "https://finite.computer";
      NEXT_PUBLIC_WORKOS_REDIRECT_URI = "https://finite.computer/callback";
      STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID = "price_1TouEuFwiFww4itkeTQNPYR6";
      FC_CORE_ADMIN_EMAILS = "paul@finite.vip,austin@finite.vip,skyler@finitesupply.xyz";
    };

    # Operator-created, root:root 0600. Variable NAMES only (values from k8s
    # Secret finite-computer-secrets on old lat1 — dashboard manifest env):
    #   FC_CORE_API_TOKEN
    #   WORKOS_API_KEY
    #   WORKOS_CLIENT_ID
    #   WORKOS_COOKIE_PASSWORD
    #   STRIPE_SECRET_KEY
    #   STRIPE_WEBHOOK_SECRET
    #   GOOGLE_WORKSPACE_CLIENT_ID        (optional in the manifest)
    #   GOOGLE_WORKSPACE_CLIENT_SECRET    (optional in the manifest)
    #   FC_RELAY_ADMIN_TOKEN              (optional; absent from the live secret)
    #   FC_RELAY_HOST_ENDPOINTS_JSON      (optional; absent from the live secret)
    environmentFiles = [ "/etc/finite/dashboard.env" ];
  };
}
