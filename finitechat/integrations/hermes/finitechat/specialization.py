"""AEON media orchestration at the Hermes-visible Finite Chat seam."""

from __future__ import annotations

import asyncio
import base64
import json
import mimetypes
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

DEFAULT_MODEL = "aeon-gemma-4-12b-k4-nvfp4-unified-fast"
MAX_LOCAL_MEDIA_BYTES = 32 * 1024 * 1024
RETRYABLE_STATUS_CODES = frozenset({408, 429, 500, 502, 503, 504})
SUPPORTED_CAPABILITIES = ("image", "audio", "video")


@dataclass(frozen=True)
class CapabilityResult:
    capability: str
    model: str
    success: bool
    text: str = ""
    error_code: str = ""
    retryable: bool = False


Requester = Callable[[str, str, dict[str, Any], float], Awaitable[tuple[int, dict[str, Any]]]]


class AeonSpecialization:
    def __init__(
        self,
        *,
        base_url: str,
        api_key: str,
        model: str = DEFAULT_MODEL,
        extra_body: dict[str, Any] | None = None,
        timeout: float = 120.0,
        requester: Requester | None = None,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.model = model
        self.extra_body = dict(extra_body or {})
        self.timeout = max(1.0, min(float(timeout), 300.0))
        self._requester = requester or _httpx_request

    @classmethod
    def from_hermes_config(cls) -> AeonSpecialization | None:
        try:
            from hermes_cli.config import cfg_get, load_config

            config = cfg_get(load_config(), "auxiliary", "vision", default={})
            base_url = str(config.get("base_url") or "").strip()
            api_key = str(config.get("api_key") or "").strip()
            if not base_url or not api_key:
                return None
            return cls(
                base_url=base_url,
                api_key=api_key,
                model=str(config.get("model") or DEFAULT_MODEL),
                extra_body=config.get("extra_body") or {},
                timeout=float(config.get("timeout") or 120),
            )
        except Exception:
            return None

    async def interpret(
        self,
        instruction: str,
        media_urls: list[str],
        media_types: list[str],
    ) -> list[CapabilityResult]:
        requests = [
            self._interpret_one(instruction, media_url, media_type)
            for media_url, media_type in zip(media_urls, media_types, strict=False)
            if _capability_for_mime(media_type) is not None
        ]
        if not requests:
            return []
        return list(await asyncio.gather(*requests))

    async def _interpret_one(
        self, instruction: str, media_url: str, media_type: str
    ) -> CapabilityResult:
        capability = _capability_for_mime(media_type)
        if capability is None:
            raise ValueError(f"unsupported specialization media type: {media_type}")
        try:
            part = await _media_part(capability, media_url, media_type)
            payload: dict[str, Any] = {
                "model": self.model,
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": instruction.strip()
                                or f"Interpret this {capability} for the user.",
                            },
                            part,
                        ],
                    }
                ],
                "stream": False,
            }
            payload.update(self.extra_body)
            status, body = await self._request(payload)
            if 200 <= status < 300:
                result = body.get("specialization_result") or {}
                return CapabilityResult(
                    capability=str(result.get("capability") or capability),
                    model=str(result.get("model") or self.model),
                    success=True,
                    text=str(result.get("text") or _choice_text(body)),
                )
            return _error_result(capability, self.model, status, body)
        except asyncio.CancelledError:
            raise
        except TimeoutError as exc:
            return CapabilityResult(
                capability=capability,
                model=self.model,
                success=False,
                text=str(exc)[:512] or "specialization request timed out",
                error_code="request_timeout",
                retryable=True,
            )
        except OSError as exc:
            return CapabilityResult(
                capability=capability,
                model=self.model,
                success=False,
                text=str(exc)[:512] or "specialization transport failed",
                error_code="upstream_error",
                retryable=True,
            )
        except Exception as exc:
            return CapabilityResult(
                capability=capability,
                model=self.model,
                success=False,
                text=str(exc)[:512],
                error_code="orchestration_error",
                retryable=False,
            )

    async def _request(self, payload: dict[str, Any]) -> tuple[int, dict[str, Any]]:
        try:
            response = await self._requester(self.base_url, self.api_key, payload, self.timeout)
        except (TimeoutError, OSError):
            return await self._requester(self.base_url, self.api_key, payload, self.timeout)
        if response[0] not in RETRYABLE_STATUS_CODES:
            return response
        return await self._requester(self.base_url, self.api_key, payload, self.timeout)


def compose_for_hermes(results: list[CapabilityResult]) -> str:
    lines = [
        "AEON specialization results follow. Preserve capability-local failures and model provenance."
    ]
    for result in results:
        identity = f"{result.capability} via {result.model}"
        if result.success:
            lines.append(f"- {identity}: {result.text}")
        else:
            code = result.error_code or "unknown_error"
            lines.append(f"- {identity} FAILED ({code}): {result.text}")
    return "\n".join(lines)


def _capability_for_mime(media_type: str) -> str | None:
    normalized = str(media_type or "").lower()
    return next(
        (capability for capability in SUPPORTED_CAPABILITIES if normalized.startswith(f"{capability}/")),
        None,
    )


async def _media_part(capability: str, media_url: str, media_type: str) -> dict[str, Any]:
    source = str(media_url or "").strip()
    if source.startswith(("https://", "http://", "data:")):
        data_url = source
    else:
        path = Path(source.removeprefix("file://"))
        size = await asyncio.to_thread(lambda: path.stat().st_size)
        if size <= 0 or size > MAX_LOCAL_MEDIA_BYTES:
            raise ValueError("local media exceeds the specialization size limit")
        content = await asyncio.to_thread(path.read_bytes)
        mime = media_type or mimetypes.guess_type(path.name)[0] or f"{capability}/octet-stream"
        data_url = f"data:{mime};base64,{base64.b64encode(content).decode('ascii')}"
    if capability == "image":
        return {"type": "image_url", "image_url": {"url": data_url}}
    if capability == "video":
        return {"type": "video_url", "video_url": {"url": data_url}}
    if source.startswith("data:"):
        encoded = source.split(",", 1)[-1]
    elif data_url.startswith("data:"):
        encoded = data_url.split(",", 1)[-1]
    else:
        return {"type": "audio_url", "audio_url": {"url": source}}
    return {
        "type": "input_audio",
        "input_audio": {"data": encoded, "format": _audio_format(media_type, source)},
    }


def _audio_format(media_type: str, source: str) -> str:
    mime = str(media_type or "").lower()
    suffix = Path(source.split("?", 1)[0]).suffix.lower()
    if "mpeg" in mime or suffix == ".mp3":
        return "mp3"
    if "ogg" in mime or "opus" in mime or suffix in {".ogg", ".opus"}:
        return "ogg"
    if "flac" in mime or suffix == ".flac":
        return "flac"
    if "mp4" in mime or suffix in {".m4a", ".mp4"}:
        return "m4a"
    return "wav"


def _choice_text(body: dict[str, Any]) -> str:
    try:
        return str(body["choices"][0]["message"]["content"])
    except (KeyError, IndexError, TypeError):
        return ""


def _error_result(
    capability: str, model: str, status: int, body: dict[str, Any]
) -> CapabilityResult:
    error = body.get("error") if isinstance(body.get("error"), dict) else {}
    return CapabilityResult(
        capability=capability,
        model=model,
        success=False,
        text=str(error.get("message") or f"specialization request failed with HTTP {status}")[:512],
        error_code=str(error.get("code") or f"http_{status}"),
        retryable=status in RETRYABLE_STATUS_CODES,
    )


async def _httpx_request(
    base_url: str, api_key: str, payload: dict[str, Any], timeout: float
) -> tuple[int, dict[str, Any]]:
    import httpx

    try:
        async with httpx.AsyncClient(timeout=timeout) as client:
            response = await client.post(
                f"{base_url}/chat/completions",
                headers={"authorization": f"Bearer {api_key}"},
                json=payload,
            )
    except httpx.TimeoutException as exc:
        raise TimeoutError("specialization request timed out") from exc
    except httpx.RequestError as exc:
        raise OSError("specialization transport failed") from exc
    try:
        body = response.json()
    except (json.JSONDecodeError, ValueError):
        body = {"error": {"code": "invalid_upstream_response", "message": "invalid JSON response"}}
    return response.status_code, body
