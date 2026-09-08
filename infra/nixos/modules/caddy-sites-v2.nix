# Dedicated Finite Sites v2 validation edge.
#
# TLS uses a Cloudflare Origin CA cert pair for finite.site and
# *.finite.site. Cloudflare proxies the names in Full (strict); the VPS does
# not need ACME or a Cloudflare API token.
{ config, ... }:
let
  originCert = "/etc/finite-saas/certs/finite-sites-v2-origin.pem";
  originKey = "/etc/finite-saas/certs/finite-sites-v2-origin.key";
  domain = config.finite.sites.baseDomain;
  sitesBackend = "reverse_proxy 127.0.0.1:8787";
in
{
  services.caddy = {
    enable = true;
    email = "paul@finite.vip";

    virtualHosts."${domain}".extraConfig = ''
      tls ${originCert} ${originKey}
      ${sitesBackend}
    '';
    virtualHosts."auth.${domain}".extraConfig = ''
      tls ${originCert} ${originKey}
      reverse_proxy 127.0.0.1:8792
    '';
    virtualHosts."*.${domain}".extraConfig = ''
      tls ${originCert} ${originKey}
      ${sitesBackend}
    '';
  };
}
