"""Opt-in native phone trial using the same managed helper and released Hermes.

Local credentials, identity, and logs stay in ~/.finite-simplex-test.
Run scripts/simplex-local; stop with Ctrl-C. No production platform changes.
"""

import argparse
import asyncio
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys

import qrcode
import yaml

import simplex_runtime as runtime


def prepare(root):
    home = root / "hermes"
    home.mkdir(parents=True, exist_ok=True, mode=0o700)
    config = home / "config.yaml"
    if not config.exists():
        source = Path.home() / ".hermes"
        source_config = yaml.safe_load((source / "config.yaml").read_text()) or {}
        model = source_config.get("model")
        if not model:
            raise RuntimeError("Configure a model in your local Hermes first")
        # Snapshot auth into this private test home; never link writable stores.
        if (source / "auth.json").exists():
            shutil.copyfile(source / "auth.json", home / "auth.json")
            (home / "auth.json").chmod(0o600)
        config.write_text(
            yaml.safe_dump(
                {
                    "model": model,
                    "gateway": {
                        "platforms": {
                            "simplex": {
                                "enabled": False,
                                "extra": {
                                    "finite_managed": True,
                                    "ws_url": runtime.WS_URL,
                                    "auto_accept": True,
                                    "dm_policy": "pairing",
                                    "group_policy": "disabled",
                                    "group_allowed": "",
                                },
                            }
                        }
                    },
                }
            )
        )
    os.environ["HERMES_HOME"] = str(home)
    os.environ["FINITECHAT_HOME"] = str(root)
    os.environ["TERMINAL_CWD"] = str(root / "workspace")
    (root / "workspace").mkdir(exist_ok=True)
    return config


async def run(root, config):
    # A second helper would clear the live PID and could stop the first trial.
    if await runtime.tcp_ready():
        raise RuntimeError("Port 5225 is in use; stop the existing SimpleX trial first")
    address = (await runtime.create_address())["address"]
    qrcode.make(address).save(root / "pairing.png")
    doc = yaml.safe_load(config.read_text())
    doc["gateway"]["platforms"]["simplex"]["enabled"] = True
    config.write_text(yaml.safe_dump(doc))
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, stop.set)
    supervisor = asyncio.create_task(runtime.supervise())
    # supervise installs its own signal handler; restore the encompassing one.
    await asyncio.sleep(0)
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, stop.set)
    gateway = None
    try:
        for _ in range(40):
            if await runtime.tcp_ready():
                break
            if supervisor.done():
                await supervisor
                raise RuntimeError("SimpleX supervisor stopped")
            await asyncio.sleep(0.25)
        else:
            raise RuntimeError("SimpleX did not start")
        with (root / "gateway.log").open("a") as log:
            gateway = await asyncio.create_subprocess_exec(
                sys.executable,
                str(Path(runtime.__file__)),
                "gateway",
                stdout=log,
                stderr=log,
                start_new_session=True,
            )
        print(
            f"Native Hermes + SimpleX running. QR: {root / 'pairing.png'}", flush=True
        )
        print(
            "Scan, send a message, then approve its code with scripts/simplex-local approve CODE.",
            flush=True,
        )
        print("Ctrl-C stops both processes and retains test state.", flush=True)
        while not stop.is_set() and gateway.returncode is None:
            try:
                await asyncio.wait_for(stop.wait(), 1)
            except TimeoutError:
                pass
        if gateway.returncode is not None:
            raise RuntimeError(f"Hermes stopped; inspect {root / 'gateway.log'}")
    finally:
        if gateway and gateway.returncode is None:
            gateway.terminate()
            try:
                await asyncio.wait_for(gateway.wait(), 15)
            except TimeoutError:
                os.killpg(gateway.pid, signal.SIGKILL)
                await gateway.wait()
        supervisor.cancel()
        try:
            await supervisor
        except asyncio.CancelledError:
            pass


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "operation", choices=["run", "status", "approve"], default="run", nargs="?"
    )
    parser.add_argument("code", nargs="?")
    args = parser.parse_args()
    root = Path(os.environ["FINITE_SIMPLEX_LOCAL_HOME"]).resolve()
    config = prepare(root)
    if args.operation == "run":
        asyncio.run(run(root, config))
    elif args.operation == "status":
        print(json.dumps(asyncio.run(runtime.status())))
    else:
        code = args.code or input("SimpleX pairing code: ")
        result = subprocess.run(
            [sys.executable, str(Path(runtime.__file__)), "approve"],
            input=code,
            text=True,
        )
        sys.exit(result.returncode)


if __name__ == "__main__":
    main()
