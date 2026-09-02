#!/usr/bin/env python3
"""Synthetic checks for the finite-lat-2 closure artifact rollout helpers."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
BUILD = ROOT / "scripts/build-lat2-nixos-closure-artifact"
DEPLOY = ROOT / "scripts/deploy-lat2-closure-cache"
INSTALL = ROOT / "scripts/install-lat2-from-artifact"

VALID_MANIFEST = {
    "schema": "finite.lat2.nixos-closure.v2",
    "host": "finite-lat-2",
    "repository": "finitecomputer/finite-mono",
    "rev": "a" * 40,
    "system": "/nix/store/" + "b" * 32 + "-nixos-system-finite-lat-2-26.05.test",
    "disko": "/nix/store/" + "c" * 32 + "-disko",
    "kexec": "/nix/store/" + "d" * 32 + "-kexec-tarball",
    "cache": "nix-cache",
}


def write_manifest(artifact: Path, payload: dict) -> None:
    (artifact / "manifest.json").write_text(json.dumps(payload) + "\n", encoding="utf-8")


def write_valid_artifact(artifact: Path) -> None:
    write_manifest(artifact, dict(VALID_MANIFEST))
    cache = artifact / "nix-cache"
    cache.mkdir()
    (cache / "nix-cache-info").write_text("StoreDir: /nix/store\n", encoding="utf-8")


def write_shim(bin_dir: Path, name: str, body: str) -> Path:
    path = bin_dir / name
    path.write_text("#!/usr/bin/env bash\n" + body, encoding="utf-8")
    path.chmod(0o755)
    return path


def write_operator_shims(bin_dir: Path, dry_activation_units: str) -> None:
    """Stand in for git/nix/ssh so the local half of the deploy script runs
    through the dry-activation fence without a network or a host."""
    write_shim(bin_dir, "git", "exit 0\n")
    write_shim(bin_dir, "nix", "exit 0\n")
    write_shim(
        bin_dir,
        "ssh",
        'case "$*" in\n'
        "  *dry-activate*) printf 'would restart the following units: %s\\n' "
        + json.dumps(dry_activation_units)
        + " ;;\n"
        "  *) cat >/dev/null ;;\n"
        "esac\n"
        "exit 0\n",
    )


def remote_activation_helpers() -> str:
    """The helper block the remote activation heredoc defines before the
    mutation boundary (allow-list, failed-unit filter, chat fold guard)."""
    source = DEPLOY.read_text(encoding="utf-8")
    begin = source.index("# --- activation helpers (begin)")
    end = source.index("# --- activation helpers (end)")
    return source[begin:end]


class Lat2ClosureArtifactTests(unittest.TestCase):
    def run_deploy(self, artifact_dir: Path, *flags: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(DEPLOY), *flags, str(artifact_dir)],
            cwd=ROOT,
            env={**os.environ, "PATH": os.environ["PATH"]},
            text=True,
            capture_output=True,
            check=False,
        )

    def run_install(self, artifact_dir: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(INSTALL), str(artifact_dir), *args],
            cwd=ROOT,
            env={**os.environ, "PATH": os.environ["PATH"]},
            text=True,
            capture_output=True,
            check=False,
        )

    def test_missing_manifest_fails_before_remote_access(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_deploy(Path(temp), "--validate-only")
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact manifest is missing", result.stderr)
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_install(Path(temp), "root@64.34.80.19", "--validate-only")
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact manifest is missing", result.stderr)

    def test_manifest_schema_and_host_are_strict(self) -> None:
        for wrong in (
            {"schema": "finite.lat3.nixos-closure.v1", "host": "finite-lat-3"},
            {"schema": "finite.lat2.nixos-closure.v2", "host": "finite-lat-3"},
        ):
            with tempfile.TemporaryDirectory() as temp:
                artifact = Path(temp)
                write_manifest(artifact, wrong)
                result = self.run_deploy(artifact, "--validate-only")
            self.assertNotEqual(result.returncode, 0)

    def test_captured_storage_ids_gate_the_build_script(self) -> None:
        # Gate B contract: the committed lat2 storage identity is captured
        # and placeholder-free, and the build guard that refuses an
        # uncaptured host is still in place (it fires again if the file
        # ever regresses or a future re-capture flips it).
        ids = ROOT / "infra/nixos/hosts/finite-lat-2/storage-ids.nix"
        source = BUILD.read_text(encoding="utf-8")
        self.assertIn("grep -q '^  captured = true;$'", source)
        self.assertTrue(ids.exists())
        ids_text = ids.read_text(encoding="utf-8")
        self.assertIn("captured = true;", ids_text)
        self.assertNotIn("captured = false", ids_text)
        self.assertNotIn("REPLACE-ME", ids_text)
        self.assertIn("/dev/disk/by-id/nvme-eui.", ids_text)
        for path in (ROOT / "infra/nixos/hosts/finite-lat-2").glob("*.nix"):
            self.assertNotIn("REPLACE-ME", path.read_text(encoding="utf-8"))

    def test_artifact_includes_the_bare_metal_install_inputs(self) -> None:
        source = BUILD.read_text(encoding="utf-8")
        # Partitioning uses the plain (unguarded-eval) disko script so the
        # build realizes exactly one disko derivation.
        self.assertIn("packages.x86_64-linux.finite-lat-2-disko", source)
        self.assertIn("finite-lat-2-kexec", source)
        self.assertIn('"disko": "$disko_path"', source)
        self.assertIn('"kexec": "$kexec_path"', source)
        self.assertEqual(source.count("nixosConfigurations.finite-lat-2."), 1)
        # No out-links: the artifact carries store paths, and each build
        # result is filtered to a single expected store path.
        self.assertNotIn("--out-link", source)
        self.assertNotIn("readlink", source)

    def test_valid_manifest_requires_file_binary_cache(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            write_manifest(artifact, dict(VALID_MANIFEST))
            result = self.run_deploy(artifact, "--validate-only")
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact cache is missing or incomplete", result.stderr)
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp)
            write_manifest(artifact, dict(VALID_MANIFEST))
            result = self.run_install(artifact, "root@64.34.80.19", "--validate-only")
        self.assertEqual(result.returncode, 66)
        self.assertIn("artifact cache is missing or incomplete", result.stderr)

    def test_invalid_system_disko_or_kexec_paths_are_refused(self) -> None:
        cases = [
            ("system", "/nix/store/" + "b" * 32 + "-nixos-system-finite-lat-1-26.05.test"),
            ("disko", "/nix/store/" + "c" * 32 + "-something-else"),
            ("kexec", "/nix/store/" + "d" * 32 + "-disko"),
        ]
        for key, value in cases:
            with tempfile.TemporaryDirectory() as temp:
                artifact = Path(temp)
                payload = {
                    **VALID_MANIFEST,
                    key: value,
                }
                write_manifest(artifact, payload)
                result = self.run_deploy(artifact, "--validate-only")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(f"invalid {key} path", result.stderr)

    def test_deploy_is_an_app_plane_rollout_with_no_runner_machinery(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        # The app-plane host runs no Agent Runner: the rollout script must
        # not carry any runner-only rollout machinery.
        self.assertNotIn("finite-saas-runner.timer", source)
        self.assertNotIn("Runner-only", source)
        # No transient-unit machinery: the only ExecStart reference is the
        # chat fold guard READING the candidate closure's own unit file.
        self.assertNotIn("systemd-run", source)
        exec_start_lines = [
            line for line in source.splitlines() if "ExecStart" in line
        ]
        self.assertEqual(exec_start_lines, ['  exec_start="$(sed -n \'s/^ExecStart=//p\' "$chat_unit_file" | head -n 1)"'])
        # The absence guard is the only runner reference allowed: a live
        # runner on the host, or any runner unit in the candidate closure,
        # refuses; an inert husk in the outgoing closure is crossable
        # because only a runner-free closure can remove it.
        self.assertIn(
            "finite-saas-runner.service is active on the app-plane host", source
        )
        self.assertIn("candidate closure contains finite-saas-runner.service", source)
        self.assertIn(
            "candidate closure contains finite-saas-runner-phala.service", source
        )

    def test_dry_activation_fences_the_declared_app_plane_unit_set(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        for unit in (
            "caddy.service",
            "finite-saas-core.service",
            "finitechat-server.service",
            "finitechat-hosted-device.service",
            "finite-saas-sites.service",
            "finite-brain-app.service",
            "finite-identity.service",
            "finite-litestream-finite-chat-server.service",
        ):
            # Declared-set membership: the unit appears as an array entry in
            # the app-plane fence, not merely as a string somewhere.
            self.assertIn(f"\n  {unit}\n", source)
        self.assertIn("--expect-startup", source)
        self.assertIn("unexpected_units", source)
        self.assertIn("refusing app-plane rollout", source)
        self.assertIn("--allow-unit", source)
        self.assertIn("approved_extra_units", source)

    def test_activation_is_fenced_and_rolls_back_the_profile(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        mutation = source.index('echo "==> mutation boundary:')
        self.assertLess(source.index("dry-activate"), mutation)
        self.assertIn("previous_system", source)
        self.assertIn("switch-to-configuration\" switch", source)
        self.assertIn("rollback", source)
        self.assertIn(
            "refusing: finite-saas-runner.service is active on the app-plane host",
            source,
        )
        self.assertIn(
            "refusing: candidate closure contains finite-saas-runner.service", source
        )

    def test_prepare_without_allow_unit_runs_the_fence(self) -> None:
        # 2026-09-02 (#799): with zero --allow-unit flags the fence's
        # approved-extra loop expanded an empty array under `set -u` and
        # --prepare died with "approved_extra_units[@]: unbound variable"
        # instead of running the fence at all. Behavior contract: the plain
        # steady-state --prepare reaches PREPARED, and the fence still
        # refuses an unapproved unit / accepts an approved one.
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            artifact = root / "artifact"
            artifact.mkdir()
            write_valid_artifact(artifact)
            shims = root / "bin"
            shims.mkdir()
            env = {**os.environ, "PATH": f"{shims}{os.pathsep}{os.environ['PATH']}"}

            def prepare(*flags: str) -> subprocess.CompletedProcess[str]:
                return subprocess.run(
                    [str(DEPLOY), "--prepare", *flags, str(artifact)],
                    cwd=ROOT,
                    env=env,
                    text=True,
                    capture_output=True,
                    check=False,
                )

            write_operator_shims(shims, "finitechat-server.service caddy.service")
            result = prepare()
            self.assertNotIn("unbound variable", result.stderr)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"==> PREPARED rev={'a' * 40}", result.stdout)

            write_operator_shims(shims, "finitechat-server.service alloy.service")
            result = prepare()
            self.assertEqual(result.returncode, 75, result.stderr)
            self.assertIn("refusing app-plane rollout", result.stderr)
            self.assertIn("alloy.service", result.stderr)

            result = prepare("--allow-unit", "alloy.service")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("explicitly-approved-unit=alloy.service", result.stdout)

    def test_every_approved_extra_units_expansion_is_set_u_safe(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        guarded = '${approved_extra_units[@]+"${approved_extra_units[@]}"}'
        self.assertIn(f"for approved_unit in {guarded}; do", source)
        unguarded = [
            line
            for line in source.splitlines()
            if '"${approved_extra_units[@]}"' in line and guarded not in line
        ]
        # The only bare expansion left is the one printf inside the
        # `${#approved_extra_units[@]} -gt 0` guard.
        self.assertEqual(len(unguarded), 1, unguarded)
        self.assertIn("explicitly-approved-unit", unguarded[0])

    def test_activation_holds_monitoring_timers_across_the_boundary(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        for timer in (
            "finite-healthcheck.timer",
            "finite-litestream-health.timer",
            "finite-hosted-web-chat-snapshot-health.timer",
            "finite-identity-backup-health.timer",
            "finite-hosted-web-chat-offsite-health.timer",
            "finite-runtime-metrics.timer",
            "finite-storage-health.timer",
        ):
            self.assertIn(f"\n  {timer}\n", source)
        # Never mask: store-symlinked units refuse it; stop + re-arm only.
        masking = [
            line
            for line in source.splitlines()
            if "systemctl mask" in line and not line.lstrip().startswith("#")
        ]
        self.assertEqual(masking, [])
        switch = source.index('"$system/bin/switch-to-configuration" switch')
        stop = source.index("\nstop_monitoring_timers\n")
        rearm = source.index("trap start_monitoring_timers EXIT")
        self.assertLess(stop, switch)
        self.assertLess(stop, rearm)
        self.assertLess(rearm, switch)
        # The re-arm is an EXIT trap so the ERR-trap rollback and the
        # roll-forward refusal restore the timers as well as success.
        self.assertIn("systemctl start \"$timer\"", source)
        self.assertIn("systemctl stop \"$timer\"", source)

    def test_trap_ignores_monitoring_only_unit_failures(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        switch = source.index('"$system/bin/switch-to-configuration" switch || switch_status=$?')
        decision = source.index('failed_outside="$(failed_units_outside_monitoring)"')
        self.assertLess(switch, decision)
        self.assertIn("(exit \"$switch_status\")", source)
        # The remote heredoc is terminated exactly once, at the end of the
        # file: a stray terminator line would run as a local command after
        # a successful deploy.
        self.assertEqual(source.count("\nLAT2\n"), 1)
        self.assertTrue(source.endswith('echo "==> DEPLOYED system=$system"\nLAT2\n'))
        self.assertIn("monitoring-only allow-list", source)
        self.assertIn("this is NOT a", source)
        self.assertIn("rollback trigger", source)
        self.assertIn("continuing post-switch verification", source)
        # A nonzero exit with NO failed unit at all is still a rollback.
        self.assertIn('[[ -n "$failed_outside" || -z "$failed_all" ]]', source)
        # Post-switch verification failures reach the trap via `abort`,
        # never a bare `exit 1` that would skip the rollback.
        tail = source[switch:]
        self.assertNotIn("exit 1", tail)
        self.assertIn('|| abort "$unit was active before and is down after a steady-state switch"', tail)
        self.assertIn('|| abort "$unit is not active after go-live activation"', tail)

    def test_trap_refuses_to_revert_a_folded_chat_database(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        self.assertIn('"$binary" rollback-check --sqlite "$sqlite"', source)
        self.assertIn("""'"fold_complete":true'""", source)
        self.assertIn("""'"rollback_allowed":false'""", source)
        rollback = source.index("rollback() {")
        self.assertIn("if chat_fold_forbids_rollback; then", source[rollback:])
        self.assertIn("REFUSING to revert to $previous_system", source)
        self.assertIn("An older binary must never serve the folded chat database", source)
        self.assertIn("ROLL FORWARD ONLY", source)
        # The binary and database come from the candidate closure's own
        # unit file, not from a hard-coded store path.
        self.assertIn('chat_unit_file="$system/etc/systemd/system/finitechat-server.service"', source)
        self.assertIn("sed -n 's/^ExecStart=//p' \"$chat_unit_file\"", source)

    def test_activation_helpers_classify_failures_and_fold_verdicts(self) -> None:
        # Behavior contract for the helpers the trap relies on, run under a
        # fake systemctl and a fake finitechat-server so the decision logic
        # is exercised rather than merely grepped.
        helpers = remote_activation_helpers()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            shims = root / "bin"
            shims.mkdir()
            write_shim(
                shims,
                "systemctl",
                'if [[ "$1" == "--failed" ]]; then printf \'%s\' "${FAKE_FAILED-}"; fi\n'
                "exit 0\n",
            )
            write_shim(
                shims,
                "finitechat-server",
                'if [[ "$1" != "rollback-check" ]]; then echo unknown >&2; exit 2; fi\n'
                '[[ -n "${FAKE_VERDICT-}" ]] || exit 1\n'
                'printf \'%s\\n\' "$FAKE_VERDICT"\n'
                '[[ "$FAKE_VERDICT" == *\'"rollback_allowed":true\'* ]]\n',
            )
            system = root / "system"
            units = system / "etc/systemd/system"
            units.mkdir(parents=True)
            sqlite = root / "server.sqlite3"
            sqlite.write_bytes(b"")
            (units / "finitechat-server.service").write_text(
                "[Service]\n"
                f"ExecStart={shims}/finitechat-server serve 127.0.0.1:8788 --sqlite {sqlite}\n",
                encoding="utf-8",
            )
            probe = root / "probe.sh"
            probe.write_text(
                "set -euo pipefail\n"
                f'system="{system}"\n'
                + helpers
                + "failed_units_outside_monitoring\n"
                "echo ---\n"
                "if chat_fold_forbids_rollback; then echo FORBID; else echo ALLOW; fi\n"
                'echo "reason=$chat_fold_reason"\n',
                encoding="utf-8",
            )

            def run(failed: str, verdict: str | None) -> str:
                env = {
                    **os.environ,
                    "PATH": f"{shims}{os.pathsep}{os.environ['PATH']}",
                    "FAKE_FAILED": failed,
                }
                if verdict is not None:
                    env["FAKE_VERDICT"] = verdict
                result = subprocess.run(
                    ["bash", str(probe)],
                    env=env,
                    text=True,
                    capture_output=True,
                    check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                return result.stdout

            monitoring_only = (
                "finite-healthcheck.service loaded failed failed Aggregate health\n"
                "finite-runtime-metrics.timer loaded failed failed Metrics\n"
            )
            forbid = json.dumps(
                {
                    "fold_complete": True,
                    "pre_fold_head": 10,
                    "current_head": 12,
                    "rollback_allowed": False,
                    "reason": "post-fold writes exist: roll forward instead",
                },
                separators=(",", ":"),
            )
            out = run(monitoring_only, forbid)
            head, _, verdict = out.partition("---\n")
            self.assertEqual(head.strip(), "")
            self.assertIn("FORBID\n", verdict)
            self.assertIn("reason=post-fold writes exist: roll forward instead", verdict)

            core_down = monitoring_only + "finitechat-server.service loaded failed failed Chat\n"
            allow = forbid.replace('"rollback_allowed":false', '"rollback_allowed":true')
            out = run(core_down, allow)
            head, _, verdict = out.partition("---\n")
            self.assertEqual(head.split(), ["finitechat-server.service"])
            self.assertIn("ALLOW\n", verdict)

            # A candidate binary without the subcommand yields no verdict:
            # the trap keeps the pre-existing revert behavior, loudly.
            out = run("", None)
            self.assertIn("ALLOW\n", out)
            self.assertIn("reason=rollback-check produced no verdict", out)

            # A closure without the chat server never consults the guard.
            (units / "finitechat-server.service").unlink()
            out = run("", forbid)
            self.assertIn("ALLOW\n", out)

    def test_go_live_requires_product_health_after_the_switch(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        for unit in (
            "finite-saas-core.service",
            "finitechat-server.service",
            "caddy.service",
            "postgresql.service",
        ):
            self.assertIn(unit, source)
        self.assertIn("--expect-startup", source)
        self.assertIn("is not active after go-live activation", source)
        self.assertIn("is down after a steady-state switch", source)

    def test_extra_units_require_explicit_cli_approval(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        self.assertIn("--allow-unit", source)
        self.assertIn("approved_extra_units", source)
        self.assertIn("invalid explicitly approved unit", source)
        self.assertNotIn(
            "alloy.service|dbus-broker.service|systemd-tmpfiles-resetup.service",
            source,
        )

    def test_install_helper_realizes_from_cache_and_drives_pinned_nixos_anywhere(self) -> None:
        source = INSTALL.read_text(encoding="utf-8")
        # The artifact cache is unsigned; the installer must read it with
        # the explicit --no-check-sigs path like every deploy-cache consumer.
        self.assertIn("--no-check-sigs", source)
        self.assertIn('nix copy --no-check-sigs --option builders \'\'', source)
        self.assertIn('--from "file://$CACHE_DIR"', source)
        self.assertIn('"$SYSTEM" "$DISKO" "$KEXEC"', source)
        self.assertIn("--store-paths \"$SYSTEM\" \"$DISKO\"", source)
        self.assertIn("--kexec \"$KEXEC\"", source)
        self.assertIn("--build-on local", source)
        self.assertIn("packages.x86_64-linux.finite-lat-2-nixos-anywhere", source)
        # Substitution from the artifact cache only: no build invocation.
        self.assertNotIn("nix build", source)

    def test_steady_state_requires_core_health_before_mutation(self) -> None:
        source = DEPLOY.read_text(encoding="utf-8")
        mutation = source.index('echo "==> mutation boundary:')
        gate = source.index("refusing steady-state switch")
        # The pre-mutation gate must exist and must run before the boundary.
        self.assertLess(gate, mutation)
        self.assertIn('"$expect_startup" != "1"', source)

    def test_app_plane_host_loads_kvm_for_the_kata_runtime(self) -> None:
        host = (ROOT / "infra/nixos/hosts/finite-lat-2/default.nix").read_text(
            encoding="utf-8"
        )
        self.assertIn('boot.kernelModules = [ "kvm-amd" ]', host)
        self.assertIn("kvm-amd kernel module", host)  # the assertion message

    def test_capture_by_id_filter_keeps_whole_namespaces_only(self) -> None:
        # The by-id listing must keep whole NVMe namespace symlinks (the EUI
        # identities Gate B commits) and drop -partN partitions and md-*
        # entries. Behavior-tested by running the script's own awk program
        # against a synthetic ls -l sample; the escaped-slash predecessor of
        # this filter matched nothing at all.
        capture = (ROOT / "infra/nixos/scripts/capture-lat2-host-evidence").read_text(
            encoding="utf-8"
        )
        match = re.search(r"awk '\$NF ~ /(.+?)/ \{print", capture)
        assert match, "by-id awk program not found in capture script"
        awk_program = match.group(1)

        sample = "\n".join(
            [
                "lrwxrwxrwx 1 root root 33 Aug 28 22:30 "
                "nvme-eui.000000000000000100a075244c213b3a -> ../../nvme0n1",
                "lrwxrwxrwx 1 root root 33 Aug 28 22:30 "
                "nvme-eui.3634473057c127620025385300000001 -> ../../nvme2n1",
                "lrwxrwxrwx 1 root root 14 Aug 28 22:30 "
                "nvme-eui.000000000000000100a075244c213b3a-part1 -> ../../nvme0n1p1",
                "lrwxrwxrwx 1 root root 13 Aug 28 22:30 "
                "md-uuid-3193acbf:4f88dd18:37a637dd:6220fcb6 -> ../../md126",
            ]
        )
        result = subprocess.run(
            ["awk", "$NF ~ /" + awk_program + "/ {print $9, \"->\", $NF}"],
            input=sample,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        kept = result.stdout.splitlines()
        self.assertEqual(len(kept), 2)
        self.assertIn("nvme-eui.000000000000000100a075244c213b3a", result.stdout)
        self.assertIn("nvme-eui.3634473057c127620025385300000001", result.stdout)
        self.assertNotIn("-part1", result.stdout)
        self.assertNotIn("md-uuid", result.stdout)

    def test_capture_parser_handles_spaced_model_names(self) -> None:
        # Behavior contract for the geometry proof: lsblk columns are
        # whitespace-padded and MODEL can contain spaces (SAMSUNG
        # MZQL21T9HCJR-00A07) — the parser must classify every disk by
        # splitting whitespace with TYPE taken from the right (lat4 hit the
        # fixed-split version of this bug: healthy disks silently dropped).
        capture = (ROOT / "infra/nixos/scripts/capture-lat2-host-evidence").read_text(
            encoding="utf-8"
        )
        match = re.search(r"<<'PY'\n(.*?)\nPY\n", capture, re.DOTALL)
        assert match, "geometry-proof python heredoc not found in capture script"
        parser_code = match.group(1)

        root_min = (935331839 + 1) * 512
        data_min = (3747612671 + 1) * 512
        sample = "\n".join(
            [
                "### block devices (bytes)",
                "nvme0n1  480103981056 Micron_7450_MTFDKBA480TFR  24454C213B3A   disk",
                "nvme1n1  480103981056 Micron_7450_MTFDKBA480TFR  24454C213BDD   disk",
                "nvme2n1 1920383410176 SAMSUNG MZQL21T9HCJR-00A07 S64GNS0WC12762 disk",
                "nvme3n1 1920383410176 SAMSUNG MZQL21T9HCJR-00A07 S64GNS0WC12751 disk",
                "### /dev/disk/by-id (whole disks only)",
            ]
        )
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as fh:
            fh.write(sample)
            sample_path = fh.name
        self.addCleanup(os.unlink, sample_path)
        result = subprocess.run(
            [sys.executable, "-", str(sample_path), str(root_min), str(data_min)],
            input=parser_code,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("found 4 disks", result.stdout)
        self.assertIn(f"root-class  (>= {root_min} bytes, < data): 2", result.stdout)
        self.assertIn(f"data-class  (>= {data_min} bytes): 2", result.stdout)
        self.assertIn("too small for the pinned root geometry: 0", result.stdout)
        self.assertIn("SAMSUNG MZQL21T9HCJR-00A07 S64GNS0WC12762", result.stdout)


if __name__ == "__main__":
    unittest.main()
