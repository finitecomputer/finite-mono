# oauth2-proxy gating brain.smoke.finite.computer, porting the smoke edge
# semantics (host-capture/smoke/web-edge.txt): Google provider, allowed email
# domain finite.vip, everything gated EXCEPT /health (the routing lives in
# modules/caddy.nix via forward_auth). Differences from smoke, deliberate:
# - /_admin is now ALSO gated (smoke's IngressRoute skipped oauth there —
#   flagged risk in infra/hosts/smoke/README.md).
# - auth happens on the brain vhost itself (no separate auth.smoke host).
{ ... }:
{
  services.oauth2-proxy = {
    enable = true;
    provider = "google";
    httpAddress = "http://127.0.0.1:4180";
    reverseProxy = true;
    setXauthrequest = true;
    email.domains = [ "finite.vip" ];
    redirectURL = "https://brain.smoke.finite.computer/oauth2/callback";
    cookie.secure = true;

    # Operator-created, root:root 0600. Variable NAMES only (values: the
    # Google OAuth client lives in the fc-auth k8s Secret on smoke; client id
    # 714116971392-1qk925pah8b7hhjr94magrtuh013bksn.apps.googleusercontent.com
    # is public, the secret + a fresh cookie secret are not):
    #   OAUTH2_PROXY_CLIENT_ID
    #   OAUTH2_PROXY_CLIENT_SECRET
    #   OAUTH2_PROXY_COOKIE_SECRET
    keyFile = "/etc/finite/oauth2-proxy.env";
  };
}
