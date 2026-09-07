# Dedicated Finite Sites v2 validation edge.
#
# TLS uses a Cloudflare Origin CA cert pair for v2.finite.chat and
# *.v2.finite.chat. Cloudflare proxies the names in Full (strict); the VPS does
# not need ACME or a Cloudflare API token.
{ ... }:
let
  originCert = "/etc/finite-saas/certs/finite-sites-v2-origin.pem";
  originKey = "/etc/finite-saas/certs/finite-sites-v2-origin.key";
  sitesBackend = "reverse_proxy 127.0.0.1:8787";
in
{
  services.caddy = {
    enable = true;
    email = "paul@finite.vip";

    virtualHosts."v2.finite.chat".extraConfig = ''
      tls ${originCert} ${originKey}
      ${sitesBackend}
    '';
    virtualHosts."*.v2.finite.chat".extraConfig = ''
      tls ${originCert} ${originKey}
      ${sitesBackend}
    '';
  };
}
