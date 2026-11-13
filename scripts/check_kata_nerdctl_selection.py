#!/usr/bin/env python3
"""Verify the default Kata nerdctl selections exclude the FIN-162 candidate.

infra/nixos/kata-nerdctl-candidate.nix patches nixpkgs-kata's nerdctl for
qualification only. A container records the nerdctl that created it as its log
collector (the nerdctl/log-uri label), so the nerdctl a Runner host or the
hosted lifecycle proof fixture selects is the collector its containers run.
This guard evaluates those selections against the nerdctl of the locked
nixpkgs-kata input:

- each Runner host's system packages (its system-path environment);
- each Runner host's finite-saas-runner unit and the PATH NixOS generates for
  it, which the Runner runs nerdctl from unless FC_RUNNER_KATA_NERDCTL_BIN
  names another binary. The unit's literal Environment entries and the
  environment files Nix renders for it must set neither PATH nor that name.
  The operator-owned environment files the unit loads are not evaluated;
- the proof fixture's tool environment, for each Linux system it is built for.

Each must reference exactly that nerdctl. The candidate must differ from it,
and neither the candidate nor its patch may appear in their derivation
closures or in a host's whole system closure.

The candidate must also still apply the repository's patch and run the Go
packages the patch adds tests to, so that building it qualifies the patch
rather than upstream nerdctl.
"""

from __future__ import annotations

import json
import re
import shlex
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HOSTS = ["finite-lat-3", "finite-lat-4", "finite-lat-5", "finite-lat-7"]
HOST_SYSTEM = "x86_64-linux"
FIXTURE = "scripts/proofs/hosted-hermes-lifecycle-tools.nix"
FIXTURE_SYSTEMS = ["x86_64-linux", "aarch64-linux"]
CANDIDATE = "infra/nixos/kata-nerdctl-candidate.nix"
PATCH = "infra/nixos/patches/nerdctl-logger-lifetime.patch"
RUNNER_UNIT = "finite-saas-runner.service"
NERDCTL_OVERRIDE = "FC_RUNNER_KATA_NERDCTL_BIN"
# Names that replace the generated PATH or the nerdctl it provides.
OVERRIDES = {NERDCTL_OVERRIDE, "PATH"}


def run(command: list[str]) -> str:
    return subprocess.run(
        command, cwd=ROOT, check=True, stdout=subprocess.PIPE, text=True
    ).stdout


def nix_list(values: list[str]) -> str:
    return "[ " + " ".join(json.dumps(value) for value in values) + " ]"


def evaluate() -> dict:
    # The baseline comes from the locked input, not from the flake's own
    # package set, so a host import switched to the candidate cannot redefine
    # what it is compared with. The patch's store path comes from the
    # repository file, as the candidate's does.
    expression = f"""
      let
        flake = builtins.getFlake {json.dumps(str(ROOT))};
        package = p: {{ drv = p.drvPath; out = p.outPath; }};
        storePath = p: "${{p}}";
        listOf = value: if builtins.isList value then value else [ value ];
        perSystem = system: let
          candidate = import (/. + {json.dumps(str(ROOT / CANDIDATE))}) {{
            nixpkgs = flake.inputs.nixpkgs-kata;
            inherit system;
          }};
        in {{
          baseline = package (import flake.inputs.nixpkgs-kata {{ inherit system; }}).nerdctl;
          candidate = package candidate;
          # What the candidate's builder is given.
          candidateBuild = {{
            doCheck = candidate.drvAttrs.doCheck or false;
            patches = map storePath (listOf (candidate.drvAttrs.patches or [ ]));
            checkPhase = candidate.drvAttrs.checkPhase or "";
          }};
        }};
        host = name: let
          config = flake.nixosConfigurations.${{name}}.config;
          runner = config.systemd.services.finite-saas-runner;
          # systemd skips a missing EnvironmentFile prefixed with "-".
          file = entry:
            if builtins.substring 0 1 entry == "-"
            then builtins.substring 1 (builtins.stringLength entry) entry
            else entry;
          etcName = path: builtins.substring 5 (builtins.stringLength path) path;
          rendered = path:
            builtins.substring 0 5 path == "/etc/" && config.environment.etc ? ${{etcName path}};
        in {{
          systemPath = config.system.path.drvPath;
          toplevel = config.system.build.toplevel.drvPath;
          runnerUnit = config.systemd.units."{RUNNER_UNIT}".unit.drvPath;
          runnerPATH = runner.environment.PATH;
          runnerEnvironment = builtins.attrNames runner.environment;
          runnerServiceEnvironment = listOf (runner.serviceConfig.Environment or [ ]);
          renderedEnvironmentFiles = builtins.listToAttrs (map (path: {{
            name = path;
            value = config.environment.etc.${{etcName path}}.text;
          }}) (builtins.filter rendered (map file (listOf (runner.serviceConfig.EnvironmentFile or [ ])))));
        }};
      in {{
        systems = builtins.listToAttrs (map (system: {{
          name = system;
          value = perSystem system;
        }}) {nix_list(FIXTURE_SYSTEMS)});
        patch = storePath (builtins.path {{
          name = {json.dumps(Path(PATCH).name)};
          path = /. + {json.dumps(str(ROOT / PATCH))};
        }});
        candidateOutput = package flake.packages.{HOST_SYSTEM}.kata-nerdctl-candidate;
        hosts = builtins.listToAttrs (map (name: {{
          inherit name;
          value = host name;
        }}) {nix_list(HOSTS)});
      }}
    """
    facts = json.loads(run(["nix", "eval", "--impure", "--json", "--expr", expression]))
    facts["fixtures"] = {
        system: run(
            [
                "nix",
                "eval",
                "--impure",
                "--raw",
                "--option",
                "eval-system",
                system,
                "--file",
                FIXTURE,
                "drvPath",
            ]
        )
        for system in FIXTURE_SYSTEMS
    }
    return facts


def package_name(store_path: str) -> str:
    # builtins.parseDrvName: the name ends at the first dash before a non-letter.
    name = Path(store_path).name.partition("-")[2].removesuffix(".drv")
    match = re.match(r"(.*?)-(?=[^A-Za-z])", name)
    return match.group(1) if match else name


def patch_test_directories() -> list[str]:
    files = re.findall(r"^\+\+\+ b/(\S+_test\.go)$", (ROOT / PATCH).read_text(), re.M)
    directories = sorted({str(Path(file).parent) for file in files})
    if not directories:
        raise SystemExit(f"{PATCH} adds no Go tests to qualify")
    return directories


def check_candidate_build(
    system: str, build: dict, patch: str, test_directories: list[str]
) -> None:
    if build["doCheck"] is not True:
        raise SystemExit(f"{system}: {CANDIDATE} no longer runs its tests (doCheck)")
    if patch not in build["patches"]:
        raise SystemExit(f"{system}: {CANDIDATE} no longer applies {patch}")
    for directory in test_directories:
        # buildGoDir (nixpkgs buildGoModule) runs `go test $checkFlags <dir>`.
        invocation = rf"^\s*buildGoDir test ['\"]?\./{re.escape(directory)}['\"]?\s*$"
        if not re.search(invocation, build["checkPhase"], re.M):
            raise SystemExit(
                f"{system}: {CANDIDATE} checkPhase no longer tests ./{directory}, "
                f"where {PATCH} adds its tests"
            )


def check_absent(label: str, drv: str, forbidden: dict[str, str]) -> None:
    requisites = set(run(["nix-store", "--query", "--requisites", drv]).split())
    for path, what in forbidden.items():
        if path in requisites:
            raise SystemExit(f"{label} depends on {what} {path}")


def check_selection(
    label: str, drv: str, baseline: str, forbidden: dict[str, str]
) -> None:
    references = run(["nix-store", "--query", "--references", drv]).split()
    selected = sorted(path for path in references if package_name(path) == "nerdctl")
    if selected != [baseline]:
        raise SystemExit(
            f"{label} selects nerdctl {selected}; expected only nixpkgs-kata's "
            f"{baseline}"
        )
    check_absent(label, drv, forbidden)


def assignment_names(assignments: list[str]) -> set[str]:
    return {
        assignment.partition("=")[0].strip()
        for assignment in assignments
        if "=" in assignment and not assignment.lstrip().startswith(("#", ";"))
    }


def check_overrides(name: str, selections: dict) -> None:
    # The generated unit environment carries the PATH checked below; any other
    # Nix-rendered PATH or nerdctl binary would replace it.
    sites = {
        f"{RUNNER_UNIT} environment": (
            set(selections["runnerEnvironment"]),
            {NERDCTL_OVERRIDE},
        ),
        f"{RUNNER_UNIT} serviceConfig.Environment": (
            assignment_names(
                [
                    assignment
                    for entry in selections["runnerServiceEnvironment"]
                    for assignment in shlex.split(entry)
                ]
            ),
            OVERRIDES,
        ),
    }
    for path, text in selections["renderedEnvironmentFiles"].items():
        if text is None:
            raise SystemExit(
                f"{name} renders {path} from a file this guard cannot read"
            )
        sites[path] = (assignment_names(text.splitlines()), OVERRIDES)
    for site, (names, forbidden) in sites.items():
        if overridden := sorted(names & forbidden):
            raise SystemExit(
                f"{name} {site} sets {', '.join(overridden)}; the Runner must run "
                f"nerdctl from the PATH NixOS generates for {RUNNER_UNIT}"
            )


def main() -> None:
    facts = evaluate()
    patch = facts["patch"]
    test_directories = patch_test_directories()
    for system, packages in facts["systems"].items():
        if packages["candidate"]["drv"] == packages["baseline"]["drv"]:
            raise SystemExit(
                f"{system}: {CANDIDATE} no longer differs from nixpkgs-kata's nerdctl"
            )
        check_candidate_build(
            system, packages["candidateBuild"], patch, test_directories
        )

    host = facts["systems"][HOST_SYSTEM]
    if facts["candidateOutput"] != host["candidate"]:
        raise SystemExit(
            f"packages.{HOST_SYSTEM}.kata-nerdctl-candidate is not {CANDIDATE}"
        )
    baseline, candidate = host["baseline"], host["candidate"]
    forbidden = {
        candidate["drv"]: "the FIN-162 candidate",
        patch: "the FIN-162 candidate's patch",
    }
    for name, selections in facts["hosts"].items():
        check_overrides(name, selections)
        check_selection(
            f"{name} system packages",
            selections["systemPath"],
            baseline["drv"],
            forbidden,
        )
        check_selection(
            f"{name} {RUNNER_UNIT}",
            selections["runnerUnit"],
            baseline["drv"],
            forbidden,
        )
        path_nerdctl = sorted(
            {
                entry.rsplit("/", 1)[0]
                for entry in selections["runnerPATH"].split(":")
                if package_name(entry.rsplit("/", 1)[0]) == "nerdctl"
            }
        )
        if path_nerdctl != [baseline["out"]]:
            raise SystemExit(
                f"{name} {RUNNER_UNIT} PATH runs nerdctl from {path_nerdctl}; "
                f"expected only {baseline['out']}"
            )
        check_absent(f"{name} system closure", selections["toplevel"], forbidden)

    for system, drv in facts["fixtures"].items():
        packages = facts["systems"][system]
        check_selection(
            f"{FIXTURE} ({system})",
            drv,
            packages["baseline"]["drv"],
            {
                packages["candidate"]["drv"]: "the FIN-162 candidate",
                patch: "the FIN-162 candidate's patch",
            },
        )

    environment_files = sorted(
        {
            path
            for selections in facts["hosts"].values()
            for path in selections["renderedEnvironmentFiles"]
        }
    )
    print(
        "Kata nerdctl selection contract: ok "
        f"(hosts and fixture select {baseline['drv']}; "
        f"candidate {candidate['drv']} with {patch} is unselected and in no host "
        f"system closure; Nix-rendered Runner env files {environment_files})"
    )


if __name__ == "__main__":
    main()
