"""Evaluated host service/transport contract; no service starts or deployment."""

import json
import subprocess


def main():
    result = subprocess.run(
        [
            "nix",
            "eval",
            "--impure",
            "--json",
            "--expr",
            """
      let
        flake = builtins.getFlake (toString ./.);
        cfg = flake.nixosConfigurations.finite-lat-2.config;
      in {
        service = cfg.systemd.services.finite-billing-metrics.serviceConfig;
        timer = cfg.systemd.timers.finite-billing-metrics.timerConfig;
        wantedBy = cfg.systemd.timers.finite-billing-metrics.wantedBy;
        importUnits = cfg.finite.importMode.units;
        alloy = cfg.environment.etc."alloy/config.alloy".text;
        oldHostHasCollector = builtins.hasAttr "finite-billing-metrics"
          flake.nixosConfigurations.finite-lat-1.config.systemd.services;
      }
    """,
        ],
        text=True,
        capture_output=True,
        check=True,
    )
    config = json.loads(result.stdout)
    service = config["service"]
    assert service["Type"] == "oneshot"
    assert service["TimeoutStartSec"] == "20s"
    assert service["ProtectSystem"] == "strict"
    assert service["NoNewPrivileges"] is True
    assert service["ReadWritePaths"] == ["/run/finite-monitoring"]
    assert service["Group"] == "finite-monitoring"
    assert service["ExecStart"].endswith(
        "/bin/finite-billing-metrics /run/finite-monitoring/finite-billing.prom"
    )
    assert config["timer"]["OnUnitActiveSec"] == "5min"
    assert config["wantedBy"] == ["timers.target"]
    for unit in ("finite-billing-metrics.service", "finite-billing-metrics.timer"):
        assert unit in config["importUnits"]
    assert config["oldHostHasCollector"] is False
    for metric in (
        "finite_billing_accounts",
        "finite_billing_collected_at_seconds",
        "finite_billing_collection_success",
    ):
        assert metric in config["alloy"]
    print("billing NixOS service and Alloy contract OK")


if __name__ == "__main__":
    main()
