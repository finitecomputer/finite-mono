# finite-identity — shared public Principal directory and trusted operator
# resolution service. Public requests use identity.finite.chat; first-party
# services use the loopback transport while preserving that public contract.
{ finitePackages, ... }:
{
  systemd.services.finite-identity = {
    description = "Finite Identity Authority";
    wants = [ "network-online.target" ];
    after = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];

    serviceConfig = {
      ExecStart = ''
        ${finitePackages.finite-identity}/bin/finite-identityd serve \
          --data /var/lib/finite-identity \
          --external-base-url https://identity.finite.chat \
          --finite-vip-domain finite.vip \
          --listen 127.0.0.1:8790 \
          --mailer resend \
          --mail-from "Finite Identity <identity@finite.chat>"
      '';
      DynamicUser = true;
      StateDirectory = "finite-identity";
      WorkingDirectory = "/var/lib/finite-identity";
      EnvironmentFile = [
        "/etc/finite/identity-operator.env"
        "/etc/finite/identity-mailer.env"
      ];
      Restart = "always";
      RestartSec = 2;
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      ProtectKernelTunables = true;
      ProtectControlGroups = true;
      RestrictSUIDSGID = true;
    };
  };
}
