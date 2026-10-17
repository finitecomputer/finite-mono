# SSH flood limits for the internet-facing Latitude hosts.
#
# External scanners hold sshd's unauthenticated-connection slots. Under the
# OpenSSH default MaxStartups of 10:30:100, sshd drops new connections at
# random once more than 10 are unauthenticated, including operator key logins.
# These limits keep a few flooding sources from filling the table and ban
# repeat abusers. There is deliberately no firewall allowlist: an operator on
# an unlisted address is still served, only throttled like any other source.
#
# Applying this restarts sshd.service (KillMode=process keeps open sessions)
# and starts fail2ban.service; nothing else is restarted on its account.
{ lib, ... }:
let
  # Operator egress addresses: fail2ban never bans them and sshd never
  # penalises them. Key authentication still applies. Update this list when
  # an operator's address changes.
  operatorSources = [ "136.49.172.158" ];
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
    PerSourcePenaltyExemptList = lib.concatStringsSep "," operatorSources;
    # The fail2ban module defaults this to VERBOSE; keep the current level.
    LogLevel = "INFO";
  };

  services.fail2ban = {
    enable = true;
    ignoreIP = operatorSources;
    maxretry = 5;
    bantime = "1h";
    bantime-increment = {
      enable = true;
      maxtime = "24h";
    };
    # Aggressive mode also counts pre-auth disconnects for valid users and
    # bare scanner connections, which make up most of the flood; normal mode
    # only counts invalid users. The ban covers port 22 only.
    jails.sshd.settings = {
      mode = "aggressive";
      findtime = "10m";
    };
  };
}
