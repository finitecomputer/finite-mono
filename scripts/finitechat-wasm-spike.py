"""Real dashboard → Rust/WASM → encrypted FiniteChat → real Hermes; local except inference."""
import argparse
import gzip
import json
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
CHAT_PATH = "/dashboard/machines/wasm-hermes/chat"


def run(*args, cwd=ROOT, env=None, stdout=None):
    subprocess.run(args, cwd=cwd, env=env, stdout=stdout, check=True)


def build():
    env = dict(os.environ, CARGO_PROFILE_RELEASE_OPT_LEVEL="s", CARGO_PROFILE_RELEASE_LTO="true",
               CARGO_PROFILE_RELEASE_CODEGEN_UNITS="1", CARGO_PROFILE_RELEASE_PANIC="abort")
    run("cargo", "build", "--locked", "-p", "finitechat-wasm", "--lib", "--target", "wasm32-unknown-unknown", "--release", env=env)
    run("wasm-bindgen", "target/wasm32-unknown-unknown/release/finitechat_wasm.wasm", "--target", "web",
        "--out-dir", str(DASHBOARD / "public/wasm-spike"))
    run("cargo", "build", "--locked", "-p", "finitechat-wasm", "--bin", "wasm-spike-relay", "-p", "finitechat-cli", "--bin", "finitechat")
    run("pnpm", "install", "--frozen-lockfile", cwd=DASHBOARD)
    for name in ["finitechat_wasm_bg.wasm", "finitechat_wasm.js"]:
        data = (DASHBOARD / "public/wasm-spike" / name).read_bytes()
        print(f"{name}: {len(data):,} bytes raw; {len(gzip.compress(data, compresslevel=9)):,} bytes gzip", flush=True)


def wait_ready(url, process):
    for _ in range(300):
        if process.poll() is not None:
            raise RuntimeError(f"Child exited with {process.returncode}; inspect run logs")
        try:
            with urllib.request.urlopen(url, timeout=1):
                return
        except (OSError, TimeoutError):
            time.sleep(0.2)
    raise RuntimeError(f"Timed out waiting for {url}")


def child_environment():
    # No ambient Core, WorkOS, or provider credentials in disposable services.
    names = {"PATH", "HOME", "TMPDIR", "TMP", "TEMP", "SHELL", "TERM", "COLORTERM", "NO_COLOR",
             "FORCE_COLOR", "LANG", "LC_ALL", "TZ", "SSL_CERT_FILE", "NIX_SSL_CERT_FILE",
             "NIX_PATH", "NIX_PROFILES", "IN_NIX_SHELL"}
    return {name: value for name, value in os.environ.items() if name in names}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["build", "up", "test"], default="up", nargs="?")
    parser.add_argument("--dashboard-port", type=int, default=13010)
    parser.add_argument("--peer-port", type=int, default=28789)
    parser.add_argument("--service-port", type=int, default=28790)
    parser.add_argument("--key-file", type=Path, default=Path("/private/tmp/finite-private-test-key.txt"))
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    if not args.skip_build:
        build()
    if args.action == "build":
        return
    # User supplies a Finite Private key for Hermes. Only Hermes receives it.
    # Destination/model match containers/agent/run_hermes_gateway.sh and
    # finitecomputer-v2/docs/service-dependencies.md's product inference endpoint.
    key = args.key_file.read_text().strip()
    if not key or "\n" in key:
        raise RuntimeError("The key file must contain just the Finite Private API key")
    for port in [args.dashboard_port, args.peer_port, args.service_port]:
        with socket.socket() as probe:
            probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            probe.bind(("127.0.0.1", port))
            probe.listen()
    hermes_package = subprocess.check_output(["nix", "build", ".#hermes-agent", "--no-link", "--print-out-paths"], cwd=ROOT, text=True).strip()
    state_root = ROOT / ".local-state/wasm-spike"
    state_root.mkdir(parents=True, exist_ok=True)
    # Darwin Unix-domain sockets have short path limits; Hermes opens one under its home.
    state = Path(tempfile.mkdtemp(prefix="fc-wasm-", dir="/tmp"))
    (state_root / state.name).symlink_to(state, target_is_directory=True)
    origin = f"http://127.0.0.1:{args.dashboard_port}"
    relay_url = f"http://127.0.0.1:{args.peer_port}"
    service_url = f"http://127.0.0.1:{args.service_port}"
    base_env = child_environment()
    env = dict(base_env, FINITECHAT_WASM_SPIKE="1", FC_WORKOS_AUTH_ENABLED="false", NODE_ENV="development",
               FINITECHAT_WASM_SPIKE_PORT=str(args.peer_port), FINITECHAT_WASM_SPIKE_URL=origin,
               FINITECHAT_WASM_SCREENSHOT=str(state / "browser-proof.png"), NEXT_DIST_DIR=".next-wasm-spike")
    processes = []

    def start(name, command, environment, cwd=ROOT):
        with (state / f"{name}.log").open("w") as log:
            process = subprocess.Popen(command, cwd=cwd, env=environment, stdout=log,
                                       stderr=subprocess.STDOUT, start_new_session=True)
        processes.append(process)
        return process

    try:
        agent_info = state / "agent-info.json"
        relay = start("relay", [str(ROOT / "target/debug/wasm-spike-relay"), str(state / "relay"),
                                str(args.peer_port), origin, str(agent_info)], base_env)
        wait_ready(f"{relay_url}/health", relay)
        with urllib.request.urlopen(f"{relay_url}/spike/user") as response:
            user_id = json.load(response)["account_id"]
        agent_home = state / "agent"
        hermes_home = state / "hermes"
        hermes_home.mkdir()
        workspace = state / "workspace"
        workspace.mkdir()
        finitechat = str(ROOT / "target/debug/finitechat")
        agent_env = dict(base_env, FINITE_HOME=str(state / "identity"), FINITECHAT_HOME=str(agent_home),
                         FINITECHAT_BIN=finitechat, HERMES_HOME=str(hermes_home),
                         FINITECHAT_WELCOME_ALLOWLIST=user_id, FINITECHAT_ALLOWED_USERS=user_id,
                         FINITECHAT_HERMES_INBOUND_STREAM="1", FINITECHAT_HERMES_SERVICE_URL=service_url,
                         FINITECHAT_HERMES_SERVICE_ADDR=f"127.0.0.1:{args.service_port}",
                         FINITE_GATEWAY_ENABLED="true", FINITE_AGENT_ID="wasm-hermes", FINITE_AGENT_NAME="Hermes")
        with agent_info.open("w") as output:
            run(finitechat, "hermes", "--agent-home", str(agent_home), "init", "--server", relay_url,
                "--device-id", "hermes-wasm-spike", "--agent-name", "Hermes", env=agent_env, stdout=output)
        with (state / "install.log").open("w") as output:
            run(finitechat, "hermes", "--agent-home", str(agent_home), "install", "--plugins-dir", str(hermes_home / "plugins"),
                "--finitechat-bin", finitechat, "--service-url", service_url, "--force", "--json", env=agent_env, stdout=output)
        config = {
            "model": {"default": "glm-5-3-flash", "provider": "custom",
                      "base_url": "https://finite-private.finite.containers.tinfoil.dev/v1",
                      "api_mode": "chat_completions", "api_key": "${FINITE_PRIVATE_API_KEY}", "context_length": 393216},
            "plugins": {"enabled": ["finitechat"]},
            "gateway": {"platforms": {"finitechat": {"enabled": True, "extra": {
                "home": str(agent_home), "finitechat_bin": finitechat, "inbound_stream": True,
                "service_url": service_url, "poll_timeout_secs": 1, "poll_limit": 10}}}},
            "terminal": {"backend": "local", "cwd": str(workspace), "persistent_shell": True},
            "approvals": {"mode": "off"}, "display": {"streaming": False},
            "security": {"redact_secrets": True}, "_config_version": 10,
        }
        (hermes_home / "config.yaml").write_text(json.dumps(config, indent=2))
        service = start("finitechat-agent", [finitechat, "hermes", "--agent-home", str(agent_home), "serve",
                         "--addr", f"127.0.0.1:{args.service_port}", "--ready-file", str(state / "service-ready.json"), "--json"], agent_env)
        wait_ready(f"{service_url}/readyz", service)
        hermes_env = dict(agent_env, FINITE_PRIVATE_API_KEY=key)
        key = ""
        start("hermes", [str(Path(hermes_package) / "bin/hermes"), "gateway", "run", "--replace"], hermes_env, workspace)
        hermes_env.pop("FINITE_PRIVATE_API_KEY")
        dashboard = start("dashboard", ["node", "node_modules/next/dist/bin/next", "dev", "--hostname",
                                       "127.0.0.1", "--port", str(args.dashboard_port)], env, DASHBOARD)
        wait_ready(f"{origin}{CHAT_PATH}", dashboard)
        (state / "ready.json").write_text(json.dumps({"dashboard": origin + CHAT_PATH, "relay": relay_url,
              "service": service_url, "pids": [process.pid for process in processes]}, indent=2))
        print(f"Open {origin}{CHAT_PATH}\nReal Hermes + Finite Private; no hosted web bridge.\nDisposable state and logs: {state}\nCtrl-C stops only this run.", flush=True)
        if args.action == "test":
            run("node", "--import", "tsx", "--test", "--test-concurrency=1", "browser/finitechat-wasm.browser.ts",
                "browser/finitechat-wasm-persistence.browser.ts", cwd=DASHBOARD, env=env)
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


if __name__ == "__main__":
    main()
