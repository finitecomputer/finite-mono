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
{ pkgs, ... }:
let
  # The legacy box1 dashboard (dashboard.finite.vip) is retired. Every Agent
  # that stayed moved to finite.computer, so the old address only explains
  # where to go. Served as a static page; no state, no upstream.
  box1RetiredNotice = pkgs.writeTextDir "index.html" ''
    <!doctype html>
    <html lang="en"><head><meta charset="utf-8">
    <meta name="viewport" content="width=device-width,initial-scale=1">
    <meta name="robots" content="noindex">
    <meta http-equiv="refresh" content="30;url=https://finite.computer">
    <title>This dashboard has moved</title>
    <style>
    :root{--bg:#f6f5f2;--card:#fff;--ink:#16161a;--muted:#5b5b66;--line:#e3e1dc;--accent:#1f4fd6;--accent-ink:#fff;--note:#eef3ff}
    @media (prefers-color-scheme: dark){:root{--bg:#121214;--card:#1c1c20;--ink:#f1f1f3;--muted:#a6a6b2;--line:#2c2c33;--accent:#5b86ff;--accent-ink:#0b0b0d;--note:#1d2540}}
    *{box-sizing:border-box}
    body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;background:var(--bg);color:var(--ink);font:17px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif;padding:24px 16px}
    main{width:100%;max-width:560px;background:var(--card);border:1px solid var(--line);border-radius:16px;padding:36px 32px}
    .tag{display:inline-block;font-size:13px;font-weight:600;letter-spacing:.04em;text-transform:uppercase;color:var(--accent);margin-bottom:10px}
    h1{font-size:30px;line-height:1.2;margin:0 0 14px}
    p{margin:0 0 14px;color:var(--muted)}
    .signin{background:var(--note);border-radius:12px;padding:16px 18px;margin:22px 0;color:var(--ink)}
    a.go{display:block;text-align:center;background:var(--accent);color:var(--accent-ink);text-decoration:none;font-weight:700;font-size:19px;padding:16px 20px;border-radius:12px;margin:8px 0 18px}
    a.go:focus-visible{outline:3px solid var(--ink);outline-offset:3px}
    .small{font-size:14px}
    </style></head>
    <body><main>
    <div class="tag">This dashboard is retired</div>
    <h1>Your agent has moved to Finite's new platform</h1>
    <p>This old dashboard has been shut down. If you had an agent here, it is waiting for you at finite.computer, with your past conversations and everything it knows.</p>
    <div class="signin">Sign in there with the same email you used here.</div>
    <a class="go" href="https://finite.computer">Go to finite.computer</a>
    <p class="small">If you chat with your agent on Telegram, nothing changes there. Taking you to finite.computer in 30 seconds.</p>
    </main></body></html>
  '';
  originCert = "/etc/finite-saas/certs/finite-chat-origin.pem";
  originKey = "/etc/finite-saas/certs/finite-chat-origin.key";
  # Hold requests while an upstream restarts during a deploy instead of
  # returning 502 at once (FIN-156). Caddy retries a failed connection for
  # every method; a request that reached the upstream is retried only if it is
  # a GET. A dead upstream still returns 502 once the window ends, so keep
  # windows short and use longer ones only for slow-restarting services.
  holdDuringRestart = window: ''
    lb_try_duration ${window}
    lb_try_interval 250ms
  '';
  sitesRedirects = ''
    # Required private exact-host map; missing/unreadable mappings fail validation.
    import /etc/finite/sites-redirects.caddy
    handle {
      header Cache-Control "no-store"
      respond "This Finite Sites address has been retired." 410
    }
  '';
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
        reverse_proxy 127.0.0.1:4200 {
          ${holdDuringRestart "30s"}
        }
      }
      @finitePrivateUserApi path /api/core/v1/finite-private/usage /api/core/v1/finite-private/usage/reset
      handle @finitePrivateUserApi {
        reverse_proxy 127.0.0.1:4200 {
          ${holdDuringRestart "30s"}
        }
      }
      handle {
        reverse_proxy 127.0.0.1:3000 {
          ${holdDuringRestart "15s"}
        }
      }
    '';

    # Canonical Brain API/signing origin. The browser Product Client remains
    # embedded through finite.computer/client so the WorkOS session cookie is
    # never broadened to sibling subdomains.
    virtualHosts."brain.finite.computer".extraConfig = ''
      reverse_proxy 127.0.0.1:3015 {
        ${holdDuringRestart "15s"}
      }
    '';

    # Canonical Finite Identity signing/API origin. The edge is dumb on
    # purpose: it proxies the service's public listener verbatim and keeps no
    # route list. Finite Identity owns its public surface in code
    # (`public_router` in finite-identity/src/authority.rs, bound on
    # 127.0.0.1:8791); operator, /internal/*, and server-to-server routes
    # exist only on the loopback full router (127.0.0.1:8790). The NIP-05
    # CORS header is set by the service, not here.
    virtualHosts."identity.finite.vip".extraConfig = ''
      reverse_proxy 127.0.0.1:8791 {
        ${holdDuringRestart "15s"}
      }
    '';

    # Public Chat listener. See modules/finitechat-server.nix.
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
        ${holdDuringRestart "30s"}
      }
    '';

    # Retired legacy box1 dashboard. Let's Encrypt (ACME), automatic, once the
    # dashboard.finite.vip record points here. Every path shows the notice.
    virtualHosts."dashboard.finite.vip".extraConfig = ''
      header Cache-Control "no-store"
      header X-Robots-Tag "noindex"
      root * ${box1RetiredNotice}
      rewrite * /index.html
      file_server
    '';

    virtualHosts."api.finite.chat".extraConfig = ''
      tls ${originCert} ${originKey}
      header Cache-Control "no-store"
      respond "Use https://finite.site with the current fsite CLI." 410
    '';
    virtualHosts."git.finite.chat".extraConfig = ''
      tls ${originCert} ${originKey}
      header Cache-Control "no-store"
      respond "Update this repository's remote using fsite auth git." 410
    '';
    virtualHosts."*.finite.chat".extraConfig = ''
      tls ${originCert} ${originKey}
      ${sitesRedirects}
    '';
    virtualHosts."*.docs.finite.chat".extraConfig = ''
      tls ${originCert} ${originKey}
      ${sitesRedirects}
    '';
  };
}
