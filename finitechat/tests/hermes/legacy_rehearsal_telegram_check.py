#!/usr/bin/env python3
"""Offline Telegram check for a full-fidelity rehearsal copy.

A rehearsal install (`install --telegram rehearsal`, the default) writes the
bot's Telegram block, pairing list and routing without the real token. This
check runs the real target gateway on a *copy* of that Hermes home against the
loopback fake Bot API (legacy_telegram_fakes.py) with a synthetic token and a
stub model, so it never reaches Telegram and never polls the bot box1 still
runs. It queues one message from each of up to `--max-users` approved people
before the gateway starts, plus one from a stranger, and reports counts only:
no user id, token or message content.

Run it inside the target Runtime image with `--network none`. It refuses a
Hermes home that holds a Telegram token.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from legacy_telegram_fakes import FakeTelegramBotApi, StubModel

CHECK_TOKEN = "4242424242:AArehearsalOnlyTokenForTheFakeBotApi_00"
STUB_REPLY = "rehearsal reply"
STRANGER_ID = 987654321
APPROVED_PROBE = r"""
import json
from gateway.pairing import PairingStore
print("APPROVED=" + json.dumps(sorted(
    str(item["user_id"]) for item in PairingStore().list_approved("telegram"))))
"""


def _env(hermes_home: Path, workdir: Path) -> dict[str, str]:
    return {
        "HOME": str(workdir / "home"),
        "HERMES_HOME": str(hermes_home),
        "PATH": os.pathsep.join([str(Path(sys.executable).parent), "/usr/bin", "/bin"]),
        "TZ": "UTC",
        "LANG": "C.UTF-8",
        "HERMES_TELEGRAM_DISABLE_FALLBACK_IPS": "1",
    }


def run_check(
    hermes_home: Path, workdir: Path, *, max_users: int = 3, timeout: float = 180
) -> dict[str, Any]:
    import yaml

    hermes_home = hermes_home.resolve()
    (workdir / "home").mkdir(parents=True, exist_ok=True)
    (workdir / "workspace").mkdir(parents=True, exist_ok=True)
    config_path = hermes_home / "config.yaml"
    config = yaml.safe_load(config_path.read_text(encoding="utf-8"))
    block = ((config.get("gateway") or {}).get("platforms") or {}).get("telegram")
    if not isinstance(block, dict):
        raise SystemExit("this Hermes home has no Telegram block to check")
    if block.get("token"):
        raise SystemExit("this Hermes home holds a Telegram token; check only a rehearsal copy")
    probe = subprocess.run(
        [sys.executable, "-B", "-c", APPROVED_PROBE],
        env=_env(hermes_home, workdir),
        cwd=workdir,
        capture_output=True,
        text=True,
        timeout=120,
    )
    if probe.returncode != 0:
        raise SystemExit("could not read the copy's Telegram pairing store")
    line = next(x for x in probe.stdout.splitlines() if x.startswith("APPROVED="))
    approved = [int(user) for user in json.loads(line.removeprefix("APPROVED=")) if user.isdigit()][
        :max_users
    ]
    summary: dict[str, Any] = {"approved_users_checked": len(approved)}
    with FakeTelegramBotApi(CHECK_TOKEN) as telegram, StubModel(STUB_REPLY) as model:
        config["model"].update(base_url=model.base_url, api_key="synthetic-loopback-key")
        finitechat = config["gateway"]["platforms"].get("finitechat")
        if isinstance(finitechat, dict):
            finitechat["enabled"] = False
        config.setdefault("plugins", {})["enabled"] = []
        block.update(enabled=True, token=CHECK_TOKEN)
        block.setdefault("extra", {})["base_url"] = telegram.base_url
        config.setdefault("terminal", {})["cwd"] = str(workdir / "workspace")
        config_path.write_text(yaml.safe_dump(config, sort_keys=False), encoding="utf-8")
        for user in approved:
            telegram.queue_message(user, "Rehearsal check: are you there?", "Owner")
        log_path = workdir / "rehearsal-gateway.log"
        started = time.time()
        with log_path.open("w") as log:
            gateway = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    "import sys; from hermes_cli.main import main; "
                    "sys.argv = ['hermes', 'gateway', 'run', '--force']; sys.exit(main())",
                ],
                env=_env(hermes_home, workdir),
                cwd=workdir / "workspace",
                stdout=log,
                stderr=subprocess.STDOUT,
            )
            try:
                first_reply = None
                answered = 0
                for user in approved:
                    replies = telegram.wait_for_message(user, timeout)
                    if replies:
                        answered += 1
                        first_reply = first_reply or time.time() - started
                telegram.queue_message(STRANGER_ID, "Who are you?", "Stranger")
                stranger = telegram.wait_for_message(STRANGER_ID, timeout)
            finally:
                gateway.terminate()
                try:
                    gateway.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    gateway.kill()
                    gateway.wait(timeout=10)
        approved_prompted = sum(
            any("pairing code" in text for text in telegram.messages_to(user)) for user in approved
        )
        summary.update(
            answered=answered,
            approved_users_prompted_for_pairing=approved_prompted,
            stranger_got_pairing_code=bool(stranger and "pairing code" in stranger[0]),
            dropped_updates=telegram.dropped_updates,
            unauthorized_calls=telegram.unauthorized_calls,
            first_reply_seconds=round(first_reply, 1) if first_reply else None,
            model_requests=len(model.requests),
        )
    summary["ok"] = (
        bool(approved)
        and summary["answered"] == len(approved)
        and summary["approved_users_prompted_for_pairing"] == 0
        and summary["stranger_got_pairing_code"]
        and summary["dropped_updates"] == 0
        and summary["unauthorized_calls"] == 0
    )
    return summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--hermes-home", type=Path, required=True, help="a COPY")
    parser.add_argument("--workdir", type=Path, required=True)
    parser.add_argument("--max-users", type=int, default=3)
    args = parser.parse_args(argv)
    summary = run_check(args.hermes_home, args.workdir, max_users=args.max_users)
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if summary["ok"] else 3


if __name__ == "__main__":
    raise SystemExit(main())
