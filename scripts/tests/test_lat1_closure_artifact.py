#!/usr/bin/env python3
"""Synthetic checks for lat1 closure artifact deploy helpers."""

from __future__ import annotations

import json
import os
import getpass
import grp
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
BUILD = ROOT / "scripts" / "build-lat1-nixos-closure-artifact"
DEPLOY = ROOT / "scripts" / "deploy-lat1-closure-cache"
PUBLISH = ROOT / "scripts" / "publish-lat1-nixos-cachix-closure"
LAT_MONITORING_SECRETS = ROOT / "infra/nixos/scripts/check-lat-monitoring-secrets"
SELECT_HARNESSES = ROOT / "scripts/ci/select-harnesses"


class Lat1ClosureArtifactTests(unittest.TestCase):
    def run_deploy(self, artifact_dir: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(DEPLOY), str(artifact_dir)],
            cwd=ROOT,
            env={**os.environ, "PATH": os.environ["PATH"]},
            text=True,
            capture_output=True,
            check=False,
        )

    def write_fake_executable(self, directory: Path, name: str, body: str) -> None:
        path = directory / name
        path.write_text(body, encoding="utf-8")
        path.chmod(0o755)

    def run_deploy_with_fake_transport(
        self,
        artifact_dir: Path,
        *,
        system_path: str,
        log_path: Path,
        extra_env: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        fake_bin = artifact_dir / "fake-bin"
        fake_bin.mkdir()
        self.write_fake_executable(
            fake_bin,
            "git",
            """#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  fetch) exit 0 ;;
  merge-base) exit 0 ;;
  cat-file) exit 1 ;;
  grep) exit 1 ;;
esac
echo "unexpected git $*" >&2
exit 2
""",
        )
        self.write_fake_executable(
            fake_bin,
            "ssh",
            """#!/usr/bin/env bash
set -euo pipefail
log="${FAKE_DEPLOY_LOG:?}"
printf 'ssh %s\\n' "$*" >> "$log"
stdin_file="$(mktemp)"
cat > "$stdin_file" || true
if grep -q 'nix-store --realise' "$stdin_file"; then
  echo remote-realise >> "$log"
fi
if grep -q -- '--option substituters' "$stdin_file"; then
  echo remote-explicit-substituter >> "$log"
fi
if grep -q -- '--option trusted-public-keys' "$stdin_file"; then
  echo remote-explicit-trusted-key >> "$log"
fi
if grep -q 'nix show-config' "$stdin_file"; then
  echo remote-cache-check >> "$log"
fi
if grep -q 'switch-to-configuration' "$stdin_file"; then
  echo remote-switch-script >> "$log"
fi
args="$*"
case "$args" in
  *"systemctl show --property=ActiveState --value"*) echo active ;;
  *"readlink -f /nix/var/nix/profiles/system"*) echo "$FAKE_SYSTEM" ;;
  *"readlink -f /run/current-system"*) echo "$FAKE_SYSTEM" ;;
  *"podman inspect finite-saas-dashboard"*) echo "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd" ;;
  *"readlink /data/recovery-snapshots/hosted-web-chat/latest"*) echo "/data/recovery-snapshots/hosted-web-chat/fake" ;;
esac
exit 0
""",
        )
        self.write_fake_executable(
            fake_bin,
            "nix",
            """#!/usr/bin/env bash
set -euo pipefail
echo "local-nix $*" >> "${FAKE_DEPLOY_LOG:?}"
exit 97
""",
        )
        self.write_fake_executable(
            fake_bin,
            "curl",
            """#!/usr/bin/env bash
set -euo pipefail
echo "curl $*" >> "${FAKE_DEPLOY_LOG:?}"
exit 0
""",
        )
        env = {
            **os.environ,
            "PATH": f"{fake_bin}:{os.environ['PATH']}",
            "FINITE_LAT1_TARGET": "finite-production-lat1",
            "FAKE_DEPLOY_LOG": str(log_path),
            "FAKE_SYSTEM": system_path,
        }
        if extra_env:
            env.update(extra_env)
        return subprocess.run(
            [str(DEPLOY), str(artifact_dir)],
            cwd=ROOT,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_missing_manifest_fails_before_git_or_nix(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_deploy(Path(temp))
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact manifest is missing", result.stderr)

    def test_manifest_schema_is_strict(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            (artifact / "manifest.json").write_text(
                json.dumps({"schema": "wrong"}) + "\n",
                encoding="utf-8",
            )
            result = self.run_deploy(artifact)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unexpected manifest schema", result.stderr)

    def test_valid_manifest_requires_file_binary_cache(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            (artifact / "manifest.json").write_text(
                json.dumps(
                    {
                        "schema": "finite.lat1.nixos-closure.v1",
                        "repository": "finitecomputer/finite-mono",
                        "rev": "a" * 40,
                        "system": "/nix/store/"
                        + "b" * 32
                        + "-nixos-system-finite-lat-1-25.11.test",
                        "disko": "/nix/store/" + "c" * 32 + "-disko",
                        "cache": "nix-cache",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            result = self.run_deploy(artifact)
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact cache is missing or incomplete", result.stderr)

    def test_cachix_manifest_realises_on_host_without_file_binary_cache(self) -> None:
        system = "/nix/store/" + "b" * 32 + "-nixos-system-finite-lat-1-26.05.test"
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            log_path = artifact / "deploy.log"
            (artifact / "manifest.json").write_text(
                json.dumps(
                    {
                        "schema": "finite.lat1.nixos-closure.v1",
                        "host": "finite-lat-1",
                        "repository": "finitecomputer/finite-mono",
                        "rev": "a" * 40,
                        "system": system,
                        "disko": "/nix/store/" + "c" * 32 + "-disko",
                        "cache": "nix-cache",
                        "transport": "cachix",
                        "cachix": {
                            "cache": "finite",
                            "substituter": "https://finite.cachix.org",
                            "trusted_public_key": "finite.cachix.org-1:Sg/y/5ax+IxMrPXS4moFro6YFdqa+a2gzDYAesRcVsk=",
                            "published": True,
                        },
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            result = self.run_deploy_with_fake_transport(
                artifact, system_path=system, log_path=log_path
            )
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            log = log_path.read_text(encoding="utf-8")
        self.assertIn("remote-cache-check", log)
        self.assertIn("remote-realise", log)
        self.assertIn("remote-explicit-substituter", log)
        self.assertIn("remote-explicit-trusted-key", log)
        self.assertNotIn("local-nix", log)

    def test_cachix_realise_only_stops_before_snapshot_or_activation(self) -> None:
        system = "/nix/store/" + "b" * 32 + "-nixos-system-finite-lat-1-26.05.test"
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            log_path = artifact / "deploy.log"
            (artifact / "manifest.json").write_text(
                json.dumps(
                    {
                        "schema": "finite.lat1.nixos-closure.v1",
                        "host": "finite-lat-1",
                        "repository": "finitecomputer/finite-mono",
                        "rev": "a" * 40,
                        "system": system,
                        "disko": "/nix/store/" + "c" * 32 + "-disko",
                        "cache": "nix-cache",
                        "transport": "cachix",
                        "cachix": {
                            "cache": "finite",
                            "substituter": "https://finite.cachix.org",
                            "trusted_public_key": "finite.cachix.org-1:Sg/y/5ax+IxMrPXS4moFro6YFdqa+a2gzDYAesRcVsk=",
                            "published": True,
                        },
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            result = self.run_deploy_with_fake_transport(
                artifact,
                system_path=system,
                log_path=log_path,
                extra_env={"FINITE_LAT1_DEPLOY_MODE": "realise-only"},
            )
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            log = log_path.read_text(encoding="utf-8")
        self.assertIn("remote-realise", log)
        self.assertIn("remote-explicit-substituter", log)
        self.assertNotIn("readlink /data/recovery-snapshots", log)
        self.assertNotIn("remote-switch-script", log)

    def test_cachix_manifest_must_be_published(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            (artifact / "manifest.json").write_text(
                json.dumps(
                    {
                        "schema": "finite.lat1.nixos-closure.v1",
                        "host": "finite-lat-1",
                        "repository": "finitecomputer/finite-mono",
                        "rev": "a" * 40,
                        "system": "/nix/store/"
                        + "b" * 32
                        + "-nixos-system-finite-lat-1-26.05.test",
                        "disko": "/nix/store/" + "c" * 32 + "-disko",
                        "cache": "nix-cache",
                        "transport": "cachix",
                        "cachix": {
                            "cache": "finite",
                            "substituter": "https://finite.cachix.org",
                            "trusted_public_key": "finite.cachix.org-1:Sg/y/5ax+IxMrPXS4moFro6YFdqa+a2gzDYAesRcVsk=",
                            "published": False,
                        },
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            result = self.run_deploy(artifact)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("published=true", result.stderr)

    def test_cachix_manifest_must_use_finite_cache(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            (artifact / "manifest.json").write_text(
                json.dumps(
                    {
                        "schema": "finite.lat1.nixos-closure.v1",
                        "host": "finite-lat-1",
                        "repository": "finitecomputer/finite-mono",
                        "rev": "a" * 40,
                        "system": "/nix/store/"
                        + "b" * 32
                        + "-nixos-system-finite-lat-1-26.05.test",
                        "disko": "/nix/store/" + "c" * 32 + "-disko",
                        "cache": "nix-cache",
                        "transport": "cachix",
                        "cachix": {
                            "cache": "other",
                            "substituter": "https://other.cachix.org",
                            "trusted_public_key": "other.cachix.org-1:abcdef",
                            "published": True,
                        },
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            result = self.run_deploy(artifact)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("finite Cachix cache", result.stderr)

    def test_build_manifest_records_cachix_metadata(self) -> None:
        source = BUILD.read_text(encoding="utf-8")
        self.assertIn('"host": "finite-lat-1"', source)
        self.assertIn('"transport": "$initial_transport"', source)
        self.assertIn("FINITE_LAT1_BUILD_FILE_CACHE", source)
        self.assertIn('"closure_size_bytes"', source)
        self.assertIn('"cachix"', source)
        self.assertIn("finite.cachix.org-1:Sg/y/5ax+IxMrPXS4moFro6YFdqa+a2gzDYAesRcVsk=", source)

    def test_publish_helper_pushes_store_paths_and_updates_manifest(self) -> None:
        self.assertTrue(PUBLISH.exists())
        source = PUBLISH.read_text(encoding="utf-8")
        self.assertIn("store-paths.txt", source)
        self.assertIn('"cachix", "push", cache_name', source)
        self.assertIn('data["transport"] = "cachix"', source)
        self.assertIn('"published": True', source)

    def test_publish_helper_pushes_and_rewrites_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            artifact = root / "artifact"
            fake_bin = root / "fake-bin"
            artifact.mkdir()
            fake_bin.mkdir()
            log_path = root / "cachix.log"
            self.write_fake_executable(
                fake_bin,
                "cachix",
                """#!/usr/bin/env bash
set -euo pipefail
printf 'cachix %s\\n' "$*" >> "${FAKE_CACHIX_LOG:?}"
""",
            )
            (artifact / "store-paths.txt").write_text(
                "\n".join(
                    [
                        "/nix/store/" + "d" * 32 + "-finite-lat1-a",
                        "/nix/store/" + "e" * 32 + "-finite-lat1-b",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            (artifact / "manifest.json").write_text(
                json.dumps(
                    {
                        "schema": "finite.lat1.nixos-closure.v1",
                        "host": "finite-lat-1",
                        "repository": "finitecomputer/finite-mono",
                        "rev": "a" * 40,
                        "system": "/nix/store/"
                        + "b" * 32
                        + "-nixos-system-finite-lat-1-26.05.test",
                        "disko": "/nix/store/" + "c" * 32 + "-disko",
                        "cache": "nix-cache",
                        "transport": "file-cache",
                        "cachix": {
                            "cache": "finite",
                            "substituter": "https://finite.cachix.org",
                            "trusted_public_key": "finite.cachix.org-1:Sg/y/5ax+IxMrPXS4moFro6YFdqa+a2gzDYAesRcVsk=",
                            "published": False,
                        },
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            result = subprocess.run(
                [str(PUBLISH), str(artifact)],
                cwd=ROOT,
                env={
                    **os.environ,
                    "PATH": f"{fake_bin}:{os.environ['PATH']}",
                    "CACHIX_AUTH_TOKEN": "fake-token",
                    "FAKE_CACHIX_LOG": str(log_path),
                },
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            log = log_path.read_text(encoding="utf-8")
            manifest = json.loads((artifact / "manifest.json").read_text(encoding="utf-8"))
        self.assertIn("cachix push finite", log)
        self.assertEqual(manifest["transport"], "cachix")
        self.assertTrue(manifest["cachix"]["published"])
        self.assertEqual(manifest["cachix"]["store_path_count"], 2)

    def test_deploy_preflights_log_shipping_secrets_before_snapshot(self) -> None:
        text = DEPLOY.read_text(encoding="utf-8")
        self.assertIn("FINITE_LOGS_WRITE_PASSWORD", text)
        self.assertIn("check-lat-monitoring-secrets", text)
        self.assertLess(
            text.index("check-lat-monitoring-secrets"),
            text.index("taking pre-deploy recovery snapshot"),
        )

    def test_ci_selector_keeps_deploy_helper_in_nix_static_lane(self) -> None:
        result = subprocess.run(
            [
                str(SELECT_HARNESSES),
                "--changed-file",
                "scripts/publish-lat1-nixos-cachix-closure",
                "--changed-file",
                "scripts/deploy-lat1-closure-cache",
                "--changed-file",
                "scripts/ci/select-harnesses",
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        selected = {
            line.removesuffix("=true")
            for line in result.stdout.splitlines()
            if line.startswith("run_") and line.endswith("=true")
        }
        self.assertEqual(selected, {"run_nix_checks"})


class LatMonitoringSecretsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name) / "root"
        self.secret_dir = self.root / "etc/finite"
        self.secret_dir.mkdir(parents=True)
        self.metrics_secret = self.secret_dir / "metrics-remote-write.env"
        self.logs_secret = self.secret_dir / "logs-write.env"
        self.metrics_secret.write_text(
            "\n".join(
                [
                    "FINITE_METRICS_REMOTE_WRITE_USERNAME=metrics-writer",
                    "FINITE_METRICS_REMOTE_WRITE_PASSWORD=metrics-secret-value",
                    "",
                ]
            ),
            encoding="utf-8",
        )
        self.logs_secret.write_text(
            "\n".join(
                [
                    "export FINITE_LOGS_WRITE_USERNAME=logs-writer",
                    "FINITE_LOGS_WRITE_PASSWORD=logs-secret-value",
                    "",
                ]
            ),
            encoding="utf-8",
        )
        self.metrics_secret.chmod(0o600)
        self.logs_secret.chmod(0o600)
        self.owner = getpass.getuser()
        self.group = grp.getgrgid(self.logs_secret.stat().st_gid).gr_name

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_checker(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                str(LAT_MONITORING_SECRETS),
                "--root",
                str(self.root),
                "--owner",
                self.owner,
                "--group",
                self.group,
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

    def combined_output(self, result: subprocess.CompletedProcess[str]) -> str:
        return result.stdout + result.stderr

    def test_valid_files_pass_without_emitting_values(self) -> None:
        result = self.run_checker()
        self.assertEqual(result.returncode, 0, result.stderr)
        output = self.combined_output(result)
        self.assertIn("no values emitted", output)
        self.assertNotIn("metrics-secret-value", output)
        self.assertNotIn("logs-secret-value", output)

    def test_missing_logs_file_fails_closed(self) -> None:
        self.logs_secret.unlink()
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing required file: /etc/finite/logs-write.env", result.stderr)

    def test_wrong_mode_and_missing_name_fail_without_emitting_value(self) -> None:
        self.logs_secret.write_text(
            "FINITE_LOGS_WRITE_USERNAME=logs-secret-value\n",
            encoding="utf-8",
        )
        self.logs_secret.chmod(0o644)
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("wrong mode for /etc/finite/logs-write.env", result.stderr)
        self.assertIn(
            "missing required variable FINITE_LOGS_WRITE_PASSWORD",
            result.stderr,
        )
        self.assertNotIn("logs-secret-value", self.combined_output(result))

    def test_malformed_entry_reports_only_file_and_line(self) -> None:
        self.metrics_secret.write_text(
            "FINITE_METRICS_REMOTE_WRITE_USERNAME=metrics-writer\n"
            "this-line-could-contain-a-secret\n",
            encoding="utf-8",
        )
        result = self.run_checker()
        self.assertEqual(result.returncode, 1)
        self.assertIn("malformed environment entry", result.stderr)
        self.assertIn("malformed environment file", result.stderr)
        self.assertNotIn("this-line-could-contain-a-secret", result.stderr)


if __name__ == "__main__":
    unittest.main()
