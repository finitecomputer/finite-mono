#!/usr/bin/env python3
"""Check preparation invariants; --artifacts-ready additionally rejects stop markers.

This is an offline artifact check, never runtime qualification or rollout approval.
"""

from __future__ import annotations

import argparse
from copy import deepcopy
from pathlib import Path
import re

import yaml


DIRECTORY = Path("infra/tinfoil/confidential-finite-private")
CANDIDATE = DIRECTORY / "tinfoil-config.inkling-2-small.candidate.yml"
BASELINE = DIRECTORY / "tinfoil-config.glm-5.3-flash.candidate.yml"
CHECKPOINT = "thinkingmachines/Inkling-2-Small@8361e7a31ae7e9966a49a91d3994fde618936c2b"
MODEL = "inkling-2-small"
ALIASES = "glm-5-3-flash,deepseek-v4-flash-0731,glm-5-2,glm-5.3-flash"
IMAGE_STOP = "REPLACE_WITH_VERIFIED_INKLING_SGLANG_IMAGE"
MPK_STOP = "REPLACE_WITH_TINFOIL_MODELWRAP_MPK"
ROOT_STOP = "REPLACE_WITH_TINFOIL_ROOT_HASH"
COMMAND = [
    "--model-path",
    f"/tinfoil/mpk/mpk-{ROOT_STOP}",
    "--served-model-name",
    MODEL,
    "--tp",
    "8",
    "--ep",
    "8",
    "--attention-backend",
    "fa4",
    "--dtype",
    "bfloat16",
    "--kv-cache-dtype",
    "bfloat16",
    "--page-size",
    "128",
    "--grammar-backend",
    "xgrammar",
    "--enable-multimodal",
    "--reasoning-parser",
    "inkling",
    "--tool-call-parser",
    "inkling2",
    "--context-length",
    "393216",
    "--host",
    "0.0.0.0",
    "--port",
    "8001",
]


class UniqueLoader(yaml.SafeLoader):
    """YAML duplicate keys must not hide a conflicting security declaration."""


def unique_mapping(loader: UniqueLoader, node: yaml.MappingNode) -> dict:
    mapping = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node)
        if key in mapping:
            raise ValueError("duplicate YAML key")
        mapping[key] = loader.construct_object(value_node)
    return mapping


UniqueLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, unique_mapping
)


def check_repository(root: Path, *, artifacts_ready: bool = False) -> list[str]:
    try:
        baseline = yaml.load((root / BASELINE).read_text(), Loader=UniqueLoader)
        candidate = yaml.load((root / CANDIDATE).read_text(), Loader=UniqueLoader)
        expected = deepcopy(baseline)
        model = candidate["models"][0]
        container = candidate["containers"][0]
        image = container["image"]
        mpk = model["mpk"]
        mpk_match = re.fullmatch(
            r"([0-9a-f]{64})_[1-9][0-9]*_[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}",
            mpk,
        )
        model_root = mpk_match[1] if mpk_match else ROOT_STOP
        expected["models"][0].update(name=MODEL, repo=CHECKPOINT, mpk=mpk)
        expected["containers"][0].update(
            name=MODEL,
            image=image,
            command=[arg.replace(ROOT_STOP, model_root) for arg in COMMAND],
        )
        for entry in expected["containers"][1]["env"]:
            for key in entry:
                if key == "UPSTREAM_BASE_URL":
                    entry[key] = f"http://{MODEL}:8001"
                elif key in {"FINITE_PRIVATE_MODEL", "FINITE_PRIVATE_UPSTREAM_MODEL"}:
                    entry[key] = MODEL
                elif key == "FINITE_PRIVATE_MODEL_ALIASES":
                    entry[key] = ALIASES
        violations = []
        if candidate != expected:
            violations.append(
                "candidate differs from the H200 recipe or retained GLM security, "
                "admission, secret, alias, network, or resource contract"
            )
        if not re.fullmatch(
            r"ghcr\.io/finitecomputer/inkling-2-small-sglang"
            r"(?::[a-zA-Z0-9._-]+)?@sha256:[0-9a-f]{64}",
            image,
        ) and (artifacts_ready or image != IMAGE_STOP):
            violations.append("verified immutable Finite Inkling image is unresolved")
        if not mpk_match and (artifacts_ready or mpk != MPK_STOP):
            violations.append(
                "checkpoint modelwrap MPK and matching mount are unresolved"
            )
        return violations
    except (OSError, ValueError, TypeError, KeyError, IndexError, yaml.YAMLError):
        return ["invalid or incomplete candidate/baseline YAML"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts-ready", action="store_true")
    args = parser.parse_args()
    violations = check_repository(
        Path(__file__).resolve().parents[1], artifacts_ready=args.artifacts_ready
    )
    for violation in violations:
        print(violation)
    if not violations:
        print("Inkling artifact contract passed; runtime qualification is separate")
    return int(bool(violations))


if __name__ == "__main__":
    raise SystemExit(main())
