from __future__ import annotations

import asyncio
import sys
import tempfile
import unittest
from pathlib import Path

PLUGIN_DIR = (
    Path(__file__).resolve().parents[2] / "integrations" / "hermes" / "finitechat"
)
sys.path.insert(0, str(PLUGIN_DIR))
from specialization import AeonSpecialization, compose_for_hermes  # noqa: E402


def success(capability: str, text: str, model: str = "aeon-test"):
    return (
        200,
        {
            "specialization_result": {
                "capability": capability,
                "model": model,
                "text": text,
            }
        },
    )


class AeonSpecializationTests(unittest.IsolatedAsyncioTestCase):
    async def test_single_media_produces_one_capability_invocation(self):
        calls = []

        async def requester(base_url, api_key, payload, timeout):
            calls.append((base_url, api_key, payload, timeout))
            return success("image", "A red square.")

        client = AeonSpecialization(
            base_url="http://worker/v1",
            api_key="worker-secret",
            model="aeon-test",
            requester=requester,
        )
        results = await client.interpret(
            "Describe it", ["https://example.com/red.png"], ["image/png"]
        )

        self.assertEqual(len(calls), 1)
        self.assertEqual(results[0].capability, "image")
        self.assertTrue(results[0].success)
        self.assertEqual(
            calls[0][2]["messages"][0]["content"][1],
            {
                "type": "image_url",
                "image_url": {"url": "https://example.com/red.png"},
            },
        )

    async def test_mixed_media_composes_success_and_capability_local_failure(self):
        calls = []

        async def requester(_base_url, _api_key, payload, _timeout):
            media_type = payload["messages"][0]["content"][1]["type"]
            calls.append(media_type)
            if media_type == "input_audio":
                return (
                    400,
                    {
                        "error": {
                            "code": "media_decode_failed",
                            "message": "audio bytes could not be decoded",
                        }
                    },
                )
            return success("image", "A chart trending upward.", "aeon-image-model")

        with tempfile.TemporaryDirectory() as directory:
            audio = Path(directory) / "clip.wav"
            audio.write_bytes(b"RIFF0000WAVE")
            client = AeonSpecialization(
                base_url="http://worker/v1",
                api_key="secret",
                requester=requester,
            )
            results = await client.interpret(
                "Compare these",
                ["https://example.com/chart.png", str(audio)],
                ["image/png", "audio/wav"],
            )

        self.assertCountEqual(calls, ["image_url", "input_audio"])
        self.assertEqual([result.success for result in results], [True, False])
        composed = compose_for_hermes(results)
        self.assertIn("image via aeon-image-model: A chart trending upward.", composed)
        self.assertIn("audio via aeon-gemma-4-12b-k4-nvfp4-unified-fast FAILED", composed)
        self.assertIn("media_decode_failed", composed)

    async def test_transient_failure_retries_once_without_switching_model(self):
        payloads = []

        async def requester(_base_url, _api_key, payload, _timeout):
            payloads.append(payload)
            if len(payloads) == 1:
                return 503, {"error": {"code": "capacity_exceeded"}}
            return success("video", "A person waves.")

        client = AeonSpecialization(
            base_url="http://worker/v1", api_key="secret", requester=requester
        )
        results = await client.interpret(
            "What happens?", ["https://example.com/clip.mp4"], ["video/mp4"]
        )

        self.assertEqual(len(payloads), 2)
        self.assertEqual(payloads[0]["model"], payloads[1]["model"])
        self.assertTrue(results[0].success)

    async def test_timeout_is_reported_after_one_retry(self):
        calls = 0

        async def requester(_base_url, _api_key, _payload, _timeout):
            nonlocal calls
            calls += 1
            raise TimeoutError("timed out")

        client = AeonSpecialization(
            base_url="http://worker/v1", api_key="secret", requester=requester
        )
        result = (
            await client.interpret(
                "Listen", ["https://example.com/clip.wav"], ["audio/wav"]
            )
        )[0]

        self.assertEqual(calls, 2)
        self.assertFalse(result.success)
        self.assertEqual(result.error_code, "request_timeout")

    async def test_cancellation_propagates_to_in_flight_capability_calls(self):
        started = asyncio.Event()
        cancelled = asyncio.Event()

        async def requester(_base_url, _api_key, _payload, _timeout):
            started.set()
            try:
                await asyncio.Event().wait()
            finally:
                cancelled.set()

        client = AeonSpecialization(
            base_url="http://worker/v1", api_key="secret", requester=requester
        )
        task = asyncio.create_task(
            client.interpret(
                "Describe", ["https://example.com/red.png"], ["image/png"]
            )
        )
        await started.wait()
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        await asyncio.wait_for(cancelled.wait(), timeout=1)


if __name__ == "__main__":
    unittest.main()
