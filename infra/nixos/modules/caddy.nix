# ONE Caddy edge for every domain on the consolidated box (replaces: lat1's
# finite.computer Caddy, lat2's *.finite.chat Caddy, clawland's Traefik for
# chat.finite.computer, and smoke's socat->Traefik chain for Brain).
#
# TLS:
# - finite.computer, brain.finite.computer, chat.finite.computer:
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

    # finite.computer: limiter-internal usage traffic plus the two narrowly
    # scoped API-key self-service routes -> Core; everything else -> dashboard.
    # Replaces the fragile hardcoded-ClusterIP Caddyfile on old lat1.
    virtualHosts."finite.computer".extraConfig = ''
      handle /internal/finite-private/* {
        reverse_proxy 127.0.0.1:4200
      }
      @finitePrivateUserApi path /api/core/v1/finite-private/usage /api/core/v1/finite-private/usage/reset
      handle @finitePrivateUserApi {
        reverse_proxy 127.0.0.1:4200
      }
      handle {
        reverse_proxy 127.0.0.1:3000
      }
    '';

    # Canonical Brain API/signing origin. The browser Product Client remains
    # embedded through finite.computer/client so the WorkOS session cookie is
    # never broadened to sibling subdomains.
    virtualHosts."brain.finite.computer".extraConfig = ''
      reverse_proxy 127.0.0.1:3015
    '';

    # Canonical Finite Identity signing/API origin. The edge is dumb on
    # purpose: it proxies the service's public listener verbatim and keeps no
    # route list. Finite Identity owns its public surface in code
    # (`public_router` in finite-identity/src/authority.rs, bound on
    # 127.0.0.1:8791); operator, /internal/*, and server-to-server routes
    # exist only on the loopback full router (127.0.0.1:8790). The NIP-05
    # CORS header is set by the service, not here.
    virtualHosts."identity.finite.vip".extraConfig = ''
      reverse_proxy 127.0.0.1:8791
    '';

    # Public URL unchanged; backend port moved 8787 -> 8788 on this box
    # (finitesitesd owns 8787). See modules/finitechat-server.nix.
    #
    # `log` enables access logging to the default logger (stderr ->
    # journald): it is the read-only evidence source for the sync-rate
    # livelock probe in scripts/finite-status (2026-08-29 chat-plane
    # outage; POST /sync/group fetch storms were invisible without it).
    # journald retention already bounds the volume; no new disk path.
    virtualHosts."chat.finite.computer".extraConfig = ''
      log
      reverse_proxy 127.0.0.1:8788 {
        # finitechat-server closes idle HTTP/1.1 connections before Caddy's
        # two-minute default. Retire pooled connections first so POSTs never
        # land on a stale upstream socket and surface as a spurious 502.
        transport http {
          keepalive 10s
        }
      }
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
  };
}
