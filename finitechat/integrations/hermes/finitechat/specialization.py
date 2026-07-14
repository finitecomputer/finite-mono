"""AEON media orchestration at the Hermes-visible Finite Chat seam."""

from __future__ import annotations

import asyncio
import base64
import json
import mimetypes
import uuid
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
    request_id: str = ""
    duration_ms: int = 0


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
        except ImportError:
            return None
        config = cfg_get(load_config(), "auxiliary", "vision", default={})
        if not isinstance(config, dict):
            raise ValueError("AEON specialization configuration must be a mapping")
        base_url = str(config.get("base_url") or "").strip()
        api_key = str(config.get("api_key") or "").strip()
        if not base_url and not api_key:
            return None
        if not base_url or not api_key:
            raise ValueError("AEON specialization requires both base_url and api_key")
        return cls(
            base_url=base_url,
            api_key=api_key,
            model=str(config.get("model") or DEFAULT_MODEL),
            extra_body=config.get("extra_body") or {},
            timeout=float(config.get("timeout") or 120),
        )

    async def interpret(
        self,
        instruction: str,
        media_urls: list[str],
        media_types: list[str],
    ) -> list[CapabilityResult]:
        requests = [self._interpret_group(instruction, group) for group in _invocation_groups(media_urls, media_types)]
        if not requests:
            return []
        return list(await asyncio.gather(*requests))

    async def _interpret_group(
        self, instruction: str, media: list[tuple[str, str, str]]
    ) -> CapabilityResult:
        capability = media[0][2]
        request_id = f"req_hermes_{uuid.uuid4().hex}"
        deadline = asyncio.get_running_loop().time() + self.timeout
        try:
            parts = await asyncio.wait_for(
                asyncio.gather(
                    *(
                        _media_part(capability, media_url, media_type)
                        for media_url, media_type, _ in media
                    )
                ),
                timeout=self._remaining(deadline),
            )
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
                            *parts,
                        ],
                    }
                ],
                "stream": False,
            }
            payload.update(self.extra_body)
            payload["_finite_request_id"] = request_id
            status, body = await self._request(payload, deadline)
            if 200 <= status < 300:
                result = body.get("specialization_result")
                if not _valid_specialization_result(result, capability):
                    return CapabilityResult(
                        capability=capability,
                        model=self.model,
                        success=False,
                        text="worker response did not contain a valid specialization result",
                        error_code="invalid_upstream_response",
                        request_id=str(body.get("request_id") or request_id),
                    )
                return CapabilityResult(
                    capability=result["capability"],
                    model=result["model"],
                    success=True,
                    text=result["text"],
                    request_id=result["request_id"],
                    duration_ms=result["duration_ms"],
                )
            return _error_result(capability, self.model, status, body, request_id)
        except asyncio.CancelledError:
            raise
        except (TimeoutError, asyncio.TimeoutError) as exc:
            return CapabilityResult(
                capability=capability,
                model=self.model,
                success=False,
                text=str(exc)[:512] or "specialization request timed out",
                error_code="request_timeout",
                retryable=True,
                request_id=request_id,
            )
        except OSError as exc:
            return CapabilityResult(
                capability=capability,
                model=self.model,
                success=False,
                text=str(exc)[:512] or "specialization transport failed",
                error_code="upstream_error",
                retryable=True,
                request_id=request_id,
            )
        except Exception as exc:
            return CapabilityResult(
                capability=capability,
                model=self.model,
                success=False,
                text=str(exc)[:512],
                error_code="orchestration_error",
                retryable=False,
                request_id=request_id,
            )

    def _remaining(self, deadline: float) -> float:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise TimeoutError("specialization request timed out")
        return remaining

    async def _request(
        self, payload: dict[str, Any], deadline: float
    ) -> tuple[int, dict[str, Any]]:
        for attempt in range(2):
            remaining = self._remaining(deadline)
            try:
                response = await asyncio.wait_for(
                    self._requester(self.base_url, self.api_key, payload, remaining),
                    timeout=remaining,
                )
            except (TimeoutError, asyncio.TimeoutError, OSError):
                if attempt == 0 and asyncio.get_running_loop().time() < deadline:
                    continue
                raise
            if response[0] not in RETRYABLE_STATUS_CODES or attempt == 1:
                return response
        raise AssertionError("bounded retry loop exhausted")


def compose_for_hermes(results: list[CapabilityResult]) -> str:
    lines = [
        "AEON specialization results follow. Preserve capability-local failures and model provenance."
    ]
    for result in results:
        request_id = getattr(result, "request_id", "")
        correlation = f" [request {request_id}]" if request_id else ""
        identity = f"{result.capability} via {result.model}"
        if result.success:
            lines.append(f"- {identity}: {result.text}{correlation}")
        else:
            code = result.error_code or "unknown_error"
            lines.append(f"- {identity} FAILED ({code}): {result.text}{correlation}")
    return "\n".join(lines)


def _capability_for_mime(media_type: str) -> str | None:
    normalized = str(media_type or "").lower()
    return next(
        (capability for capability in SUPPORTED_CAPABILITIES if normalized.startswith(f"{capability}/")),
        None,
    )


def _invocation_groups(
    media_urls: list[str], media_types: list[str]
) -> list[list[tuple[str, str, str]]]:
    groups: list[list[tuple[str, str, str]]] = []
    image_group: list[tuple[str, str, str]] | None = None
    for media_url, media_type in zip(media_urls, media_types):
        capability = _capability_for_mime(media_type)
        if capability is None:
            continue
        item = (media_url, media_type, capability)
        if capability == "image":
            if image_group is None:
                image_group = []
                groups.append(image_group)
            image_group.append(item)
        else:
            groups.append([item])
    return groups


def unavailable_results(media_types: list[str], model: str = DEFAULT_MODEL) -> list[CapabilityResult]:
    return [
        CapabilityResult(
            capability=group[0][2],
            model=model,
            success=False,
            text="AEON specialization is not configured",
            error_code="capability_unavailable",
            retryable=False,
        )
        for group in _invocation_groups([""] * len(media_types), media_types)
    ]


def _valid_specialization_result(result: Any, expected_capability: str) -> bool:
    return (
        isinstance(result, dict)
        and result.get("capability") == expected_capability
        and isinstance(result.get("text"), str)
        and isinstance(result.get("model"), str)
        and bool(result["model"])
        and isinstance(result.get("request_id"), str)
        and bool(result["request_id"])
        and isinstance(result.get("duration_ms"), int)
        and result["duration_ms"] >= 0
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
    capability: str,
    model: str,
    status: int,
    body: dict[str, Any],
    fallback_request_id: str,
) -> CapabilityResult:
    error = body.get("error") if isinstance(body.get("error"), dict) else {}
    return CapabilityResult(
        capability=capability,
        model=model,
        success=False,
        text=str(error.get("message") or f"specialization request failed with HTTP {status}")[:512],
        error_code=str(error.get("code") or f"http_{status}"),
        retryable=status in RETRYABLE_STATUS_CODES,
        request_id=str(body.get("request_id") or fallback_request_id),
    )


async def _httpx_request(
    base_url: str, api_key: str, payload: dict[str, Any], timeout: float
) -> tuple[int, dict[str, Any]]:
    import httpx

    wire_payload = dict(payload)
    request_id = str(wire_payload.pop("_finite_request_id", ""))
    headers = {"authorization": f"Bearer {api_key}"}
    if request_id:
        headers["x-request-id"] = request_id
    try:
        async with httpx.AsyncClient(timeout=timeout) as client:
            response = await client.post(
                f"{base_url}/chat/completions",
                headers=headers,
                json=wire_payload,
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
