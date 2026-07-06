from __future__ import annotations

import asyncio
import base64
import contextlib
import io
import json
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path
from typing import Any

from tests.hermes.test_live_hermes_agent_media_e2e import (
    FINITECHAT_BIN,
    FINITECHAT_SERVER_BIN,
    JsonSmokeReport,
    RecordingPluginContext,
    free_local_port,
    load_adapter_module,
    run_json,
    wait_for_health,
)
from tests.hermes.test_live_ios_simulator_hermes_media_e2e import (
    AGENT_MEDIA_CAPTION,
    AGENT_TEXT,
    IOS_CAPTION,
    PNG_1X1,
    ios_agent_reply_summary,
    ios_state_has_agent_replies,
)

REPO_ROOT = Path(__file__).resolve().parents[2]
BUNDLE_ID = os.environ.get("FINITECHAT_IOS_BUNDLE_ID", "computer.finite.finitechat")
IOS_DEVICE_IDENTITY = "ios-hermes-media-phone"
DEFAULT_IOS_DEVICE_MEDIA_REPORT = REPO_ROOT / "target/ios-device-hermes-agent-media-e2e/report.json"


def run_cmd(args: list[str], *, timeout: int = 180) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
    if result.returncode != 0:
        raise AssertionError(
            f"command failed:\n  args={args!r}\n  stdout={result.stdout}\n  stderr={result.stderr}"
        )
    return result


def run_devicectl_json(args: list[str], *, timeout: int = 180) -> dict[str, Any]:
    with tempfile.NamedTemporaryFile(prefix="finite-devicectl-", suffix=".json") as output:
        run_cmd(["xcrun", "devicectl", *args, "--json-output", output.name], timeout=timeout)
        return json.loads(Path(output.name).read_text())


def available_device_identifier() -> str:
    explicit = os.environ.get("FINITECHAT_IOS_DEVICE_ID") or os.environ.get("IOS_DEVICE_ID")
    if explicit:
        return explicit
    value = run_devicectl_json(["list", "devices"], timeout=30)
    for device in value.get("result", {}).get("devices", []):
        state = str(device.get("connectionProperties", {}).get("tunnelState", "")).lower()
        name = str(device.get("name") or "")
        identifier = str(device.get("identifier") or "")
        if identifier and "unavailable" not in state and name:
            return identifier
    raise AssertionError("no available paired iPhone found; set FINITECHAT_IOS_DEVICE_ID")


def mac_lan_ip() -> str:
    explicit = os.environ.get("FINITECHAT_IOS_DEVICE_SERVER_HOST")
    if explicit:
        return explicit
    for interface in ("en0", "en1"):
        result = subprocess.run(
            ["ipconfig", "getifaddr", interface], capture_output=True, text=True
        )
        candidate = result.stdout.strip()
        if result.returncode == 0 and candidate and not candidate.startswith("127."):
            return candidate
    result = run_cmd(["ifconfig"], timeout=30)
    for line in result.stdout.splitlines():
        fields = line.strip().split()
        if len(fields) >= 2 and fields[0] == "inet" and not fields[1].startswith("127."):
            return fields[1]
    raise AssertionError("could not determine Mac LAN IP; set FINITECHAT_IOS_DEVICE_SERVER_HOST")


def assert_app_installed(device: str) -> None:
    value = run_devicectl_json(
        ["device", "info", "apps", "--device", device, "--bundle-id", BUNDLE_ID],
        timeout=60,
    )
    apps = value.get("result", {}).get("apps", [])
    if not any(app.get("bundleIdentifier") == BUNDLE_ID for app in apps):
        raise AssertionError(
            f"{BUNDLE_ID} is not installed on {device}; run the physical product harness or install from Xcode first"
        )


def find_process_identifier(value: Any) -> int | None:
    if isinstance(value, dict):
        for key, child in value.items():
            normalized = "".join(ch for ch in key.lower() if ch.isalnum())
            if normalized in {"pid", "processid", "processidentifier"}:
                with contextlib.suppress(TypeError, ValueError):
                    return int(child)
        for child in value.values():
            found = find_process_identifier(child)
            if found is not None:
                return found
    if isinstance(value, list):
        for child in value:
            found = find_process_identifier(child)
            if found is not None:
                return found
    return None


def launch_phone_app(device: str, launch_args: list[str]) -> int:
    try:
        value = run_devicectl_json(
            [
                "device",
                "process",
                "launch",
                "--device",
                device,
                "--terminate-existing",
                BUNDLE_ID,
                *launch_args,
            ],
            timeout=90,
        )
    except AssertionError as error:
        raise AssertionError(
            f"failed to launch {BUNDLE_ID} on {device}; unlock the phone and leave it awake, then rerun. {error}"
        ) from error
    pid = find_process_identifier(value)
    if pid is None:
        raise AssertionError(f"devicectl launch did not report a process identifier: {value!r}")
    return pid


def terminate_phone_app(device: str, pid: int) -> None:
    subprocess.run(
        [
            "xcrun",
            "devicectl",
            "device",
            "process",
            "terminate",
            "--device",
            device,
            "--pid",
            str(pid),
            "--kill",
        ],
        capture_output=True,
        text=True,
        timeout=60,
    )


def pull_phone_store(device: str, store_path: Path) -> None:
    support_root = store_path.parent
    support_root.mkdir(parents=True, exist_ok=True)
    pull_parent = support_root / ".device-pull-FiniteChatStore"
    shutil.rmtree(pull_parent, ignore_errors=True)
    pull_parent.mkdir(parents=True)
    run_cmd(
        [
            "xcrun",
            "devicectl",
            "device",
            "copy",
            "from",
            "--device",
            device,
            "--domain-type",
            "appDataContainer",
            "--domain-identifier",
            BUNDLE_ID,
            "--source",
            "Library/Application Support/FiniteChatStore",
            "--destination",
            str(pull_parent),
        ],
        timeout=120,
    )
    copied_store = pull_parent / "FiniteChatStore"
    if copied_store.is_dir():
        source = copied_store
    elif (pull_parent / "client.sqlite3").is_file():
        source = pull_parent
    else:
        raise AssertionError(f"devicectl copy did not produce FiniteChatStore under {pull_parent}")
    shutil.rmtree(store_path, ignore_errors=True)
    shutil.move(str(source), str(store_path))
    shutil.rmtree(pull_parent, ignore_errors=True)


def read_phone_state(store_path: Path, server_url: str) -> dict[str, Any]:
    return run_json(
        [
            str(FINITECHAT_BIN),
            "app",
            "--data-dir",
            str(store_path),
            "--server",
            server_url,
            "--device-id",
            IOS_DEVICE_IDENTITY,
            "state",
        ],
        timeout=30,
    )


@unittest.skipUnless(
    os.environ.get("FINITE_IOS_DEVICE_HERMES_AGENT_MEDIA_E2E") == "1",
    "run scripts/ios-device-hermes-agent-media-e2e.sh to enable this physical-phone e2e",
)
class LiveIosDeviceHermesMediaE2ETest(unittest.IsolatedAsyncioTestCase):
    async def test_ios_device_exchanges_image_media_with_real_hermes_agent(self) -> None:
        self.assertTrue(FINITECHAT_BIN.exists(), f"missing {FINITECHAT_BIN}")
        self.assertTrue(FINITECHAT_SERVER_BIN.exists(), f"missing {FINITECHAT_SERVER_BIN}")
        device = available_device_identifier()
        assert_app_installed(device)
        smoke = JsonSmokeReport(
            "ios_device_hermes_agent_media_e2e",
            "FINITE_IOS_DEVICE_HERMES_AGENT_MEDIA_E2E_REPORT",
            DEFAULT_IOS_DEVICE_MEDIA_REPORT,
        )
        smoke.fact("platform", "ios_device")
        smoke.fact("bundle_id", BUNDLE_ID)
        smoke.fact("ios_device_id", IOS_DEVICE_IDENTITY)
        smoke.fact("device_identifier", device)

        with tempfile.TemporaryDirectory(prefix="finite-ios-device-hermes-media-") as tmp_value:
            tmp = Path(tmp_value)
            store_path = tmp / "FiniteChatStore"
            port = free_local_port()
            server_url = f"http://{mac_lan_ip()}:{port}"
            server_bind = f"0.0.0.0:{port}"

            with (tmp / "server.log").open("w") as server_log:
                server = subprocess.Popen(
                    [
                        str(FINITECHAT_SERVER_BIN),
                        "serve",
                        server_bind,
                        "--sqlite",
                        str(tmp / "server.sqlite3"),
                    ],
                    stdout=server_log,
                    stderr=subprocess.STDOUT,
                    text=True,
                )
                try:
                    started = time.monotonic()
                    wait_for_health(f"http://127.0.0.1:{port}/health")
                    smoke.step("server_ready", started)
                    await self._run_phone_round_trip(tmp, store_path, server_url, device, smoke)
                    smoke.finish()
                finally:
                    server.terminate()
                    with contextlib.suppress(subprocess.TimeoutExpired):
                        server.wait(timeout=5)
                    if server.poll() is None:
                        server.kill()

    async def _run_phone_round_trip(
        self,
        tmp: Path,
        store_path: Path,
        server_url: str,
        device: str,
        smoke: JsonSmokeReport,
    ) -> None:
        from gateway.config import PlatformConfig

        agent_home = tmp / "agent-home"
        started = time.monotonic()
        await asyncio.to_thread(
            run_json,
            [
                str(FINITECHAT_BIN),
                "hermes",
                "--home",
                str(agent_home),
                "init",
                "--server",
                server_url,
            ],
        )
        smoke.step("agent_init", started)
        adapter = self._build_adapter(agent_home, PlatformConfig)
        agent_received = asyncio.get_running_loop().create_future()

        async def handle_agent_message(event):
            chat_id = getattr(getattr(event, "source", None), "chat_id", None)
            received = {
                "text": getattr(event, "text", ""),
                "message_type": str(getattr(event, "message_type", "")).split(".")[-1].lower(),
                "media_types": list(getattr(event, "media_types", []) or []),
                "chat_id": chat_id,
            }
            if chat_id:
                agent_image = tmp / "agent-reply.png"
                agent_image.write_bytes(PNG_1X1)
                text_result = await adapter.send(chat_id, AGENT_TEXT)
                media_result = await adapter.send_image_file(
                    chat_id,
                    str(agent_image),
                    caption=AGENT_MEDIA_CAPTION,
                )
                received["agent_text_send_success"] = bool(getattr(text_result, "success", False))
                received["agent_media_send_success"] = bool(getattr(media_result, "success", False))
                received["agent_media_send_error"] = getattr(media_result, "error", None)
            if not agent_received.done():
                agent_received.set_result(received)
            return None

        if hasattr(adapter, "set_message_handler"):
            adapter.set_message_handler(handle_agent_message)
        else:
            adapter.handle_message = handle_agent_message

        with contextlib.redirect_stdout(io.StringIO()):
            started = time.monotonic()
            connected = await adapter.connect()
            smoke.step("adapter_connect", started)
        self.assertTrue(connected)
        smoke.fact("adapter_inbound_stream", bool(getattr(adapter, "inbound_stream", False)))
        smoke.fact("adapter_service_url_present", bool(getattr(adapter, "service_url", "")))
        pid: int | None = None
        try:
            pin_info = await asyncio.to_thread(
                run_json,
                [str(FINITECHAT_BIN), "hermes", "--home", str(agent_home), "pin"],
            )
            smoke.fact("invite_url_present", bool(pin_info.get("url")))
            smoke.fact("pin_present", bool(pin_info.get("pin")))
            launch_args = [
                "--finitechat-server",
                server_url,
                "--finitechat-device",
                IOS_DEVICE_IDENTITY,
                "--finitechat-persist-launch-config",
                "--finitechat-auto-join",
                pin_info["url"],
                "--finitechat-pin",
                pin_info["pin"],
                "--finitechat-auto-send-attachment-base64",
                base64.b64encode(PNG_1X1).decode("ascii"),
            ]
            launch_args.extend(
                [
                    "--finitechat-auto-send-attachment-filename",
                    "ios-device-image.png",
                    "--finitechat-auto-send-attachment-mime-type",
                    "image/png",
                    "--finitechat-auto-send-attachment-caption",
                    IOS_CAPTION,
                ]
            )
            started = time.monotonic()
            pid = await asyncio.to_thread(launch_phone_app, device, launch_args)
            smoke.step("ios_device_app_launch", started)

            started = time.monotonic()
            received = await asyncio.wait_for(agent_received, timeout=120)
            smoke.step("agent_receive_ios_media", started)
            self.assertEqual(received["text"], IOS_CAPTION)
            self.assertEqual(received["message_type"], "photo")
            self.assertEqual(received["media_types"], ["image/png"])
            self.assertTrue(received["agent_text_send_success"])
            self.assertTrue(
                received["agent_media_send_success"], received["agent_media_send_error"]
            )
            smoke.fact("agent_received_media_types", received["media_types"])

            deadline = time.monotonic() + 75
            last_state: dict[str, Any] | None = None
            started = time.monotonic()
            while time.monotonic() < deadline:
                if pid is not None:
                    terminate_phone_app(device, pid)
                    pid = None
                await asyncio.to_thread(pull_phone_store, device, store_path)
                last_state = await asyncio.to_thread(read_phone_state, store_path, server_url)
                if ios_state_has_agent_replies(last_state):
                    summary = ios_agent_reply_summary(last_state)
                    smoke.step("ios_device_store_receives_agent_replies", started)
                    smoke.fact("ios_received_text", summary["texts"])
                    smoke.fact("ios_received_media_count", summary["media_reply_count"])
                    return
                pid = await asyncio.to_thread(
                    launch_phone_app,
                    device,
                    [
                        "--finitechat-server",
                        server_url,
                        "--finitechat-device",
                        IOS_DEVICE_IDENTITY,
                        "--finitechat-persist-launch-config",
                    ],
                )
                await asyncio.sleep(3)

            self.fail(
                "iOS device app store did not persist both Hermes replies; "
                f"messages={[(m.get('text'), len(m.get('media') or [])) for m in (last_state or {}).get('messages', [])]!r}"
            )
        finally:
            if pid is not None:
                terminate_phone_app(device, pid)
            await adapter.disconnect()

    def _build_adapter(self, agent_home: Path, platform_config):
        module = load_adapter_module()
        ctx = RecordingPluginContext()
        module.register(ctx)
        factory = ctx.entries[0]["adapter_factory"]
        try:
            config = platform_config(
                enabled=True,
                extra={
                    "home": str(agent_home),
                    "finitechat_bin": str(FINITECHAT_BIN),
                    "poll_timeout_secs": 1,
                    "inbound_stream": True,
                },
            )
        except TypeError:
            import types

            config = types.SimpleNamespace(
                enabled=True,
                extra={
                    "home": str(agent_home),
                    "finitechat_bin": str(FINITECHAT_BIN),
                    "poll_timeout_secs": 1,
                    "inbound_stream": True,
                },
            )
        return factory(config)


if __name__ == "__main__":
    unittest.main()
