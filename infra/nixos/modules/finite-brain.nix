# finite-brain — first-party server on the canonical brain.finite.computer
# origin and behind the finite.computer dashboard's embedded client proxy. It
# binds loopback only. WorkOS protects the embedded Product Client; Brain owns
# route-level auth through signed Nostr request proofs.
{ finitePackages, ... }:
{
  systemd.services.finite-brain-app = {
    description = "FiniteBrain Rust application server";
    wants = [ "network-online.target" ];
    after = [
      "network-online.target"
      "finite-identity.service"
      "finite-saas-core.service"
    ];
    requires = [
      "finite-identity.service"
      "finite-saas-core.service"
    ];
    wantedBy = [ "multi-user.target" ];

    environment = {
      FBRAIN_CONFIG_DIR = "/var/lib/finitebrain/fbrain";
      FINITE_BRAIN_ADDR = "127.0.0.1:3015";
      FINITE_BRAIN_DB = "/var/lib/finitebrain/finite-brain.sqlite3";
      FINITE_BRAIN_PUBLIC_BASE_URL = "https://brain.finite.computer";
      FINITE_BRAIN_SERVER_URL = "https://brain.finite.computer";
      FINITE_IDENTITY_AUTHORITY = "http://127.0.0.1:8790";
      FC_CORE_API_BASE_URL = "http://127.0.0.1:4200";
    };

    serviceConfig = {
      ExecStart = "${finitePackages.finite-brain}/bin/finite-brain";
      EnvironmentFile = [
        "/etc/finite/identity-operator.env"
        "/etc/finite/brain-authority.env"
      ];
      DynamicUser = true;
      # SQLite restored from smoke at cutover; real path under DynamicUser:
      # /var/lib/private/finitebrain/finite-brain.sqlite3.
      StateDirectory = "finitebrain";
      WorkingDirectory = "/var/lib/finitebrain";
      Restart = "always";
      RestartSec = 3;
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectSystem = "full";
      ReadWritePaths = [ "/var/lib/finitebrain" ];
    };
  };
}
