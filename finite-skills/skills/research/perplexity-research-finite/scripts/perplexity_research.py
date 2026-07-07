#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import sys
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

PERPLEXITY_SEARCH_URL = "https://api.perplexity.ai/search"
PERPLEXITY_SONAR_URL = "https://api.perplexity.ai/v1/sonar"
FIRECRAWL_SCRAPE_URL = "https://api.firecrawl.dev/v2/scrape"


def load_hermes_env() -> None:
    home = os.path.expanduser("~")
    env_path = os.path.join(home, ".hermes", ".env")
    if not os.path.exists(env_path):
        return
    try:
        with open(env_path, "r", encoding="utf-8") as handle:
            for raw in handle:
                line = raw.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                key, value = line.split("=", 1)
                if key and key not in os.environ:
                    os.environ[key] = value
    except OSError:
        return


def env_key(name: str) -> str:
    load_hermes_env()
    value = os.getenv(name, "").strip()
    if not value:
        print(f"{name} is not set.", file=sys.stderr)
        raise SystemExit(2)
    return value


def post_json(url: str, body: dict[str, Any], *, headers: dict[str, str]) -> dict[str, Any]:
    request = Request(
        url,
        data=json.dumps(body).encode("utf-8"),
        headers={"Content-Type": "application/json", **headers},
        method="POST",
    )
    try:
        with urlopen(request) as response:
            return json.loads(response.read().decode("utf-8"))
    except HTTPError as exc:
        payload = exc.read().decode("utf-8", errors="replace")
        print(payload or f"HTTP {exc.code}", file=sys.stderr)
        raise SystemExit(exc.code)
    except URLError as exc:
        print(str(exc), file=sys.stderr)
        raise SystemExit(1)


def perplexity_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {env_key('PERPLEXITY_API_KEY')}"}


def firecrawl_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {env_key('FIRECRAWL_API_KEY')}"}


def compact_results(payload: dict[str, Any]) -> list[dict[str, Any]]:
    return payload.get("results") or payload.get("search_results") or []


def render_search_markdown(payload: dict[str, Any], query: str) -> str:
    results = compact_results(payload)
    lines = [f"# Perplexity Search: {query}", ""]
    if not results:
        lines.append("No results.")
        return "\n".join(lines)

    for index, result in enumerate(results, start=1):
        title = result.get("title") or result.get("name") or result.get("url") or "Untitled"
        url = result.get("url") or ""
        lines.append(f"## {index}. {title}")
        if url:
            lines.append(url)
        date = result.get("date") or result.get("last_updated")
        snippet = result.get("snippet") or result.get("content") or ""
        if date:
            lines.append(f"- Date: {date}")
        if snippet:
            lines.append(f"- Snippet: {snippet}")
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def render_brief_markdown(payload: dict[str, Any], query: str) -> str:
    choice = ((payload.get("choices") or [{}])[0] or {}).get("message") or {}
    answer = choice.get("content") or ""
    citations = payload.get("citations") or []
    results = payload.get("search_results") or []
    related_questions = payload.get("related_questions") or []

    lines = [f"# Perplexity Brief: {query}", ""]
    if answer:
        lines.extend([answer, ""])

    if citations:
        lines.append("## Citations")
        for url in citations:
            lines.append(f"- {url}")
        lines.append("")

    if results:
        lines.append("## Search Results")
        for result in results:
            title = result.get("title") or result.get("url") or "Untitled"
            url = result.get("url") or ""
            snippet = result.get("snippet") or ""
            lines.append(f"- {title}")
            if url:
                lines.append(f"  {url}")
            if snippet:
                lines.append(f"  {snippet}")
        lines.append("")

    if related_questions:
        lines.append("## Related Questions")
        for question in related_questions:
            lines.append(f"- {question}")
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def render_fetch_markdown(payload: dict[str, Any], url: str) -> str:
    data = payload.get("data") or {}
    markdown = data.get("markdown") or data.get("content") or ""
    metadata = data.get("metadata") or {}

    lines = [f"# Firecrawl Fetch: {url}", ""]
    if metadata:
        lines.append("## Metadata")
        for key in ("title", "sourceURL", "statusCode", "description"):
            value = metadata.get(key)
            if value:
                lines.append(f"- {key}: {value}")
        lines.append("")

    lines.append("## Content")
    lines.append("")
    lines.append(markdown or "No markdown returned.")
    lines.append("")
    return "\n".join(lines)


def build_search_payload(args: argparse.Namespace) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "query": args.query,
        "max_results": args.max_results,
    }
    if args.domain:
        payload["search_domain_filter"] = args.domain
    if args.recency:
        payload["search_recency_filter"] = args.recency
    if args.region:
        payload["country"] = args.region.upper()
    return payload


def build_brief_payload(args: argparse.Namespace) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "model": "sonar-pro",
        "messages": [
            {
                "role": "system",
                "content": "Give a grounded, concise research brief. Prefer direct factual claims, say when evidence is mixed, and rely on live sources.",
            },
            {"role": "user", "content": args.query},
        ],
    }
    if args.domain:
        payload["search_domain_filter"] = args.domain
    if args.recency:
        payload["search_recency_filter"] = args.recency
    if args.search_mode:
        payload["search_mode"] = args.search_mode
    if args.related_questions:
        payload["return_related_questions"] = True
    return payload


def build_fetch_payload(args: argparse.Namespace) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "url": args.url,
        "formats": ["markdown"],
        "onlyMainContent": True,
    }
    if args.fresh:
        payload["maxAge"] = 0
    return payload


def cmd_search(args: argparse.Namespace) -> int:
    payload = post_json(
        PERPLEXITY_SEARCH_URL,
        build_search_payload(args),
        headers=perplexity_headers(),
    )
    if args.json:
        print(json.dumps(payload, indent=2))
    else:
        print(render_search_markdown(payload, args.query), end="")
    return 0


def cmd_brief(args: argparse.Namespace) -> int:
    payload = post_json(
        PERPLEXITY_SONAR_URL,
        build_brief_payload(args),
        headers=perplexity_headers(),
    )
    if args.json:
        print(json.dumps(payload, indent=2))
    else:
        print(render_brief_markdown(payload, args.query), end="")
    return 0


def cmd_fetch(args: argparse.Namespace) -> int:
    payload = post_json(
        FIRECRAWL_SCRAPE_URL,
        build_fetch_payload(args),
        headers=firecrawl_headers(),
    )
    if args.json:
        print(json.dumps(payload, indent=2))
    else:
        print(render_fetch_markdown(payload, args.url), end="")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Perplexity + Firecrawl research helper")
    subparsers = parser.add_subparsers(dest="command", required=True)

    search = subparsers.add_parser("search", help="Return raw Perplexity search results")
    search.add_argument("--query", required=True)
    search.add_argument("--domain", action="append", default=[], help="Repeat to limit results to specific domains")
    search.add_argument("--recency", choices=["day", "week", "month", "year"])
    search.add_argument("--region", help="Optional country code such as US")
    search.add_argument("--max-results", type=int, default=8)
    search.add_argument("--json", action="store_true")
    search.set_defaults(func=cmd_search)

    brief = subparsers.add_parser("brief", help="Return a cited Sonar Pro brief plus source URLs")
    brief.add_argument("--query", required=True)
    brief.add_argument("--domain", action="append", default=[], help="Repeat to limit results to specific domains")
    brief.add_argument("--recency", choices=["day", "week", "month", "year"])
    brief.add_argument("--search-mode", choices=["web", "academic"], default="web")
    brief.add_argument("--related-questions", action="store_true")
    brief.add_argument("--json", action="store_true")
    brief.set_defaults(func=cmd_brief)

    fetch = subparsers.add_parser("fetch", help="Fetch exact source text with Firecrawl")
    fetch.add_argument("--url", required=True)
    fetch.add_argument("--fresh", action="store_true", help="Bypass Firecrawl cache")
    fetch.add_argument("--json", action="store_true")
    fetch.set_defaults(func=cmd_fetch)

    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
