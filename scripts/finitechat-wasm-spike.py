"""Disposable local WASM/Native interoperability harness; run via the sibling script."""
import argparse
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
DASHBOARD = ROOT / "finitecomputer-v2/apps/dashboard"


def run(*args, cwd=ROOT, env=None):
    subprocess.run(args, cwd=cwd, env=env, check=True)


def build():
    run("cargo", "build", "--locked", "-p", "finitechat-wasm", "--lib", "--target", "wasm32-unknown-unknown")
    run("wasm-bindgen", "target/wasm32-unknown-unknown/debug/finitechat_wasm.wasm", "--target", "web",
        "--out-dir", str(DASHBOARD / "public/wasm-spike"))
    run("cargo", "build", "--locked", "-p", "finitechat-wasm", "--bin", "wasm-spike-peer")
    run("pnpm", "install", "--frozen-lockfile", cwd=DASHBOARD)


def wait_ready(url, process):
    for _ in range(300):
        if process.poll() is not None:
            raise RuntimeError(f"Child process exited with {process.returncode}; inspect the run logs")
        try:
            with urllib.request.urlopen(url, timeout=1):
                return
        except (OSError, TimeoutError):
            time.sleep(0.2)
    raise RuntimeError(f"Timed out waiting for {url}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["build", "up", "test"], default="up", nargs="?")
    parser.add_argument("--dashboard-port", type=int, default=13010)
    parser.add_argument("--peer-port", type=int, default=28789)
    args = parser.parse_args()
    build()
    if args.action == "build":
        return
    for port in [args.dashboard_port, args.peer_port]:
        # Refuse existing listeners; never stop or borrow somebody else's stack.
        with socket.socket() as probe:
            # Like the real listeners, permit reuse after a prior run's TIME_WAIT.
            probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            probe.bind(("127.0.0.1", port))
            probe.listen()
    state_root = ROOT / ".local-state/wasm-spike"
    state_root.mkdir(parents=True, exist_ok=True)
    state = Path(tempfile.mkdtemp(prefix="run-", dir=state_root))
    origin = f"http://127.0.0.1:{args.dashboard_port}"
    env = dict(os.environ, FINITECHAT_WASM_SPIKE="1", FC_WORKOS_AUTH_ENABLED="false",
               FINITECHAT_WASM_SPIKE_PORT=str(args.peer_port), FINITECHAT_WASM_SPIKE_URL=origin,
               FINITECHAT_WASM_SCREENSHOT=str(state / "browser-proof.png"),
               NEXT_DIST_DIR=".next-wasm-spike")
    processes = []
    try:
        with (state / "peer.log").open("w") as log:
            peer = subprocess.Popen([str(ROOT / "target/debug/wasm-spike-peer"), str(state / "peer"),
                                     str(args.peer_port), origin], cwd=ROOT, env=env,
                                    stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        processes.append(peer)
        wait_ready(f"http://127.0.0.1:{args.peer_port}/health", peer)
        with (state / "dashboard.log").open("w") as log:
            dashboard = subprocess.Popen(["node", "node_modules/next/dist/bin/next", "dev", "--hostname",
                                          "127.0.0.1", "--port", str(args.dashboard_port)], cwd=DASHBOARD,
                                         env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        processes.append(dashboard)
        wait_ready(f"{origin}/wasm-spike", dashboard)
        print(f"Open {origin}/wasm-spike\nDisposable state and logs: {state}\nCtrl-C stops only this run.", flush=True)
        if args.action == "test":
            run("node", "--import", "tsx", "--test", "browser/finitechat-wasm.browser.ts", cwd=DASHBOARD, env=env)
        else:
            while all(process.poll() is None for process in processes):
                time.sleep(0.5)
            raise RuntimeError(f"A spike process exited; inspect {state}")
    except KeyboardInterrupt:
        pass
    finally:
        for process in reversed(processes):
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
        for process in processes:
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        # Retain synthetic state and logs for inspection; never reset an existing run.


if __name__ == "__main__":
    main()
