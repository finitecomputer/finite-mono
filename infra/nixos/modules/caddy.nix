# ONE Caddy edge for every domain on the consolidated box (replaces: lat1's
# finite.computer Caddy, lat2's *.finite.chat Caddy, clawland's Traefik for
# chat.finite.computer, smoke's socat->Traefik chain for brain).
#
# TLS:
# - finite.computer, chat.finite.computer, brain.smoke.finite.computer:
#   Let's Encrypt (ACME), automatic.
# - *.finite.chat / *.docs.finite.chat / api.finite.chat: Cloudflare Origin
#   CA cert pair at /etc/finite-saas/certs/finite-chat-origin.{pem,key},
#   copied from lat2 at cutover (host-agnostic — covers the zone). The .key
#   must be root:caddy 0640 as on lat2. Cloudflare proxies the zone in Full
#   (strict); no ACME, no CF API token on the box.
{ ... }:
let
  originCert = "/etc/finite-saas/certs/finite-chat-origin.pem";
  originKey = "/etc/finite-saas/certs/finite-chat-origin.key";
  sitesBackend = "reverse_proxy 127.0.0.1:8787";
in
{
  services.caddy = {
    enable = true;
    email = "paul@finite.vip"; # ACME account (matches the legacy fleet's)

    # finite.computer: /internal/finite-private/* -> core (the limiter's
    # usage API), everything else -> dashboard. Replaces the fragile
    # hardcoded-ClusterIP Caddyfile on old lat1.
    virtualHosts."finite.computer".extraConfig = ''
      handle /internal/finite-private/* {
        reverse_proxy 127.0.0.1:4200
      }
      handle {
        reverse_proxy 127.0.0.1:3000
      }
    '';

    # Public URL unchanged; backend port moved 8787 -> 8788 on this box
    # (finitesitesd owns 8787). See modules/finitechat-server.nix.
    virtualHosts."chat.finite.computer".extraConfig = ''
      reverse_proxy 127.0.0.1:8788
    '';

    virtualHosts."api.finite.chat".extraConfig = ''
      tls ${originCert} ${originKey}
      ${sitesBackend}
    '';
    virtualHosts."*.finite.chat".extraConfig = ''
      tls ${originCert} ${originKey}
      ${sitesBackend}
    '';
    virtualHosts."*.docs.finite.chat".extraConfig = ''
      tls ${originCert} ${originKey}
      ${sitesBackend}
    '';

    # brain vhosts DEFERRED with the brain + oauth2-proxy modules to the
    # auth-integration follow-up (Paul, 2026-07-09). Brain stays on smoke;
    # brain.smoke.finite.computer keeps serving there. Do NOT add a
    # brain.smoke or brain.finite vhost here until finite-brain.nix +
    # oauth2-proxy.nix are re-imported — a vhost with no backend would fail
    # ACME and 502. The forward_auth/oauth2 config is preserved in git
    # history and in those modules' comments for the follow-up.
  };
}
