# oauth2-proxy gating brain.smoke.finite.computer, porting the smoke edge
# semantics (host-capture/smoke/web-edge.txt): Google provider, allowed email
# domain finite.vip, everything gated EXCEPT /health (the routing lives in
# modules/caddy.nix via forward_auth). Differences from smoke, deliberate:
# - /_admin is now ALSO gated (smoke's IngressRoute skipped oauth there —
#   flagged risk in infra/hosts/smoke/README.md).
# - auth happens on the brain vhost itself (no separate auth.smoke host).
{ pkgs, ... }:
{
  services.oauth2-proxy = {
    enable = true;
    # The pinned nixpkgs (2026-06-30) defaults Go to 1.25, but oauth2-proxy
    # 7.15.3's go.mod requires >= 1.26 — the stock package fails to build in
    # the sandbox (GOTOOLCHAIN=local can't fetch a newer Go). Build it with
    # go_1_26, which the same nixpkgs already provides. Drop this override
    # when a nixpkgs bump makes the default Go >= 1.26.
    package = pkgs.oauth2-proxy.override {
      buildGoModule = pkgs.buildGoModule.override { go = pkgs.go_1_26; };
    };
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
