# SSH flood limits for the internet-facing Latitude hosts.
#
# External scanners hold sshd's unauthenticated-connection slots. Under the
# OpenSSH default MaxStartups of 10:30:100, sshd drops new connections at
# random once more than 10 are unauthenticated, including operator key logins.
# These sshd settings keep a few flooding sources from filling the table and
# refuse repeat offenders for at most 10 minutes. There is deliberately no
# firewall allowlist and no ban daemon: an operator on an unlisted address is
# still served, only throttled like any other source.
#
# Applying this restarts sshd.service (KillMode=process keeps open sessions);
# nothing else is restarted on its account.
{ lib, ... }:
let
  # Sources sshd never penalises; key authentication still applies. The
  # private WireGuard overlay, and the operator's current egress address.
  # That address changes every few days and nothing depends on it being
  # current: a stale entry only loses the exemption.
  exemptSources = [
    "10.254.3.0/29"
    "136.49.172.158"
  ];
in
{
  services.openssh.settings = {
    # Random early drop starts at 64 unauthenticated connections (was 10) and
    # every new one is refused at 256.
    MaxStartups = "64:30:256";
    # One source address may hold at most 10 unauthenticated connections.
    # PerSourcePenaltyExemptList does not apply to this limit, so it must stay
    # above the operator's own concurrent connection setups.
    PerSourceMaxStartups = 10;
    # Unauthenticated connections are closed after 30 s instead of 120 s.
    LoginGraceTime = 30;
    # OpenSSH defaults except noauth (default 1 s), so sources that connect
    # and close without authenticating are refused sooner. The default max of
    # 10 minutes still bounds every penalty.
    PerSourcePenalties = "noauth:5";
    PerSourcePenaltyExemptList = lib.concatStringsSep "," exemptSources;
    # Pinned so a module that raises it (fail2ban defaults it to VERBOSE)
    # cannot change log volume unnoticed.
    LogLevel = "INFO";
  };
}
