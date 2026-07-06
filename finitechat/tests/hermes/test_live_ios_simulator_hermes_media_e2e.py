from __future__ import annotations

import asyncio
import base64
import contextlib
import io
import json
import os
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

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_IOS_MEDIA_REPORT = REPO_ROOT / "target/ios-hermes-agent-media-e2e/report.json"
FINITECHAT_RMP_BIN = Path(
    os.environ.get("FINITECHAT_RMP_BIN", REPO_ROOT / "target/debug/finitechat-rmp")
)
BUNDLE_ID = os.environ.get("FINITECHAT_IOS_BUNDLE_ID", "computer.finite.finitechat")
IOS_DEVICE_ID = "ios-hermes-media-sim"
IOS_CAPTION = "ios media hello"
AGENT_TEXT = f"agent text echo: {IOS_CAPTION}"
AGENT_MEDIA_CAPTION = "agent media echo"
PNG_1X1 = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMB/6X7z9kAAAAASUVORK5CYII="
)


def run_cmd(args: list[str], *, timeout: int = 180) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
    if result.returncode != 0:
        raise AssertionError(
            f"command failed:\n  args={args!r}\n  stdout={result.stdout}\n  stderr={result.stderr}"
        )
    return result


def booted_simulator_udid() -> str:
    explicit = os.environ.get("IOS_SIMULATOR_UDID")
    if explicit:
        return explicit
    result = run_cmd(["xcrun", "simctl", "list", "devices", "booted", "-j"], timeout=30)
    data = json.loads(result.stdout)
    for runtimes in data.get("devices", {}).values():
        for device in runtimes:
            if device.get("state") == "Booted" and device.get("isAvailable", True):
                return str(device["udid"])
    raise AssertionError("boot an iOS Simulator or set IOS_SIMULATOR_UDID")


def launch_ios_app(
    *,
    udid: str,
    support_root: Path,
    server_url: str,
    invite_url: str,
    pin: str,
    image_path: Path,
) -> None:
    run_cmd([str(FINITECHAT_RMP_BIN), "run", "ios", "--udid", udid], timeout=600)
    subprocess.run(
        ["xcrun", "simctl", "terminate", udid, BUNDLE_ID], capture_output=True, text=True
    )
    run_cmd(
        [
            "xcrun",
            "simctl",
            "launch",
            udid,
            BUNDLE_ID,
            "--finitechat-product-harness-root",
            str(support_root),
            "--finitechat-server",
            server_url,
            "--finitechat-device",
            IOS_DEVICE_ID,
            "--finitechat-auto-join",
            invite_url,
            "--finitechat-pin",
            pin,
            "--finitechat-auto-send-attachment-file",
            str(image_path),
            "--finitechat-auto-send-attachment-caption",
            IOS_CAPTION,
        ],
        timeout=60,
    )


def read_ios_app_state(support_root: Path, server_url: str) -> dict[str, Any]:
    return run_json(
        [
            str(FINITECHAT_BIN),
            "app",
            "--data-dir",
            str(support_root / "FiniteChatStore"),
            "--server",
            server_url,
            "--device-id",
            IOS_DEVICE_ID,
            "state",
        ],
        timeout=30,
    )


def ios_state_has_agent_replies(state: dict[str, Any]) -> bool:
    summary = ios_agent_reply_summary(state)
    return (
        AGENT_TEXT in summary["texts"]
        and AGENT_MEDIA_CAPTION in summary["texts"]
        and summary["media_reply_count"] >= 1
    )


def ios_agent_reply_summary(state: dict[str, Any]) -> dict[str, Any]:
    texts = [message.get("text") or "" for message in state.get("messages", [])]
    media_reply_count = 0
    for message in state.get("messages", []):
        if message.get("text") == AGENT_MEDIA_CAPTION and message.get("media"):
            media_reply_count += len(message.get("media") or [])
    return {"texts": texts, "media_reply_count": media_reply_count}


@unittest.skipUnless(
    os.environ.get("FINITE_IOS_HERMES_AGENT_MEDIA_E2E") == "1",
    "run scripts/ios-hermes-agent-media-e2e.sh to build and enable this e2e",
)
class LiveIosSimulatorHermesMediaE2ETest(unittest.IsolatedAsyncioTestCase):
    async def test_ios_simulator_exchanges_image_media_with_real_hermes_agent(self) -> None:
        self.assertTrue(FINITECHAT_BIN.exists(), f"missing {FINITECHAT_BIN}")
        self.assertTrue(FINITECHAT_SERVER_BIN.exists(), f"missing {FINITECHAT_SERVER_BIN}")
        self.assertTrue(FINITECHAT_RMP_BIN.exists(), f"missing {FINITECHAT_RMP_BIN}")
        udid = booted_simulator_udid()
        smoke = JsonSmokeReport(
            "ios_simulator_hermes_agent_media_e2e",
            "FINITE_IOS_HERMES_AGENT_MEDIA_E2E_REPORT",
            DEFAULT_IOS_MEDIA_REPORT,
        )
        smoke.fact("platform", "ios_simulator")
        smoke.fact("bundle_id", BUNDLE_ID)
        smoke.fact("ios_device_id", IOS_DEVICE_ID)
        smoke.fact("simulator_udid", udid)

        with tempfile.TemporaryDirectory(prefix="finite-ios-hermes-media-") as tmp_value:
            tmp = Path(tmp_value)
            support_root = tmp / "ios-support"
            support_root.mkdir()
            ios_image = tmp / "ios-image.png"
            agent_image = tmp / "agent-reply.png"
            ios_image.write_bytes(PNG_1X1)
            agent_image.write_bytes(PNG_1X1)
            server_url = f"http://127.0.0.1:{free_local_port()}"

            with (tmp / "server.log").open("w") as server_log:
                server = subprocess.Popen(
                    [
                        str(FINITECHAT_SERVER_BIN),
                        "serve",
                        server_url.removeprefix("http://"),
                        "--sqlite",
                        str(tmp / "server.sqlite3"),
                    ],
                    stdout=server_log,
                    stderr=subprocess.STDOUT,
                    text=True,
                )
                try:
                    started = time.monotonic()
                    wait_for_health(f"{server_url}/health")
                    smoke.step("server_ready", started)
                    await self._run_ios_round_trip(
                        tmp, support_root, server_url, agent_image, ios_image, udid, smoke
                    )
                    smoke.finish()
                finally:
                    subprocess.run(
                        ["xcrun", "simctl", "terminate", udid, BUNDLE_ID],
                        capture_output=True,
                        text=True,
                    )
                    server.terminate()
                    with contextlib.suppress(subprocess.TimeoutExpired):
                        server.wait(timeout=5)
                    if server.poll() is None:
                        server.kill()

    async def _run_ios_round_trip(
        self,
        tmp: Path,
        support_root: Path,
        server_url: str,
        agent_image: Path,
        ios_image: Path,
        udid: str,
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
        try:
            pin_info = await asyncio.to_thread(
                run_json,
                [str(FINITECHAT_BIN), "hermes", "--home", str(agent_home), "pin"],
            )
            smoke.fact("invite_url_present", bool(pin_info.get("url")))
            smoke.fact("pin_present", bool(pin_info.get("pin")))
            started = time.monotonic()
            await asyncio.to_thread(
                launch_ios_app,
                udid=udid,
                support_root=support_root,
                server_url=server_url,
                invite_url=pin_info["url"],
                pin=pin_info["pin"],
                image_path=ios_image,
            )
            smoke.step("ios_app_launch", started)

            started = time.monotonic()
            received = await asyncio.wait_for(agent_received, timeout=90)
            smoke.step("agent_receive_ios_media", started)
            self.assertEqual(received["text"], IOS_CAPTION)
            self.assertEqual(received["message_type"], "photo")
            self.assertEqual(received["media_types"], ["image/png"])
            self.assertTrue(received["agent_text_send_success"])
            self.assertTrue(
                received["agent_media_send_success"], received["agent_media_send_error"]
            )
            smoke.fact("agent_received_media_types", received["media_types"])

            deadline = time.monotonic() + 45
            last_state: dict[str, Any] | None = None
            started = time.monotonic()
            while time.monotonic() < deadline:
                last_state = await asyncio.to_thread(read_ios_app_state, support_root, server_url)
                if ios_state_has_agent_replies(last_state):
                    summary = ios_agent_reply_summary(last_state)
                    smoke.step("ios_receive_agent_replies", started)
                    smoke.fact("ios_received_text", summary["texts"])
                    smoke.fact("ios_received_media_count", summary["media_reply_count"])
                    return
                await asyncio.sleep(1)

            self.fail(
                "iOS app store did not persist both Hermes replies; "
                f"messages={[(m.get('text'), len(m.get('media') or [])) for m in (last_state or {}).get('messages', [])]!r}"
            )
        finally:
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
