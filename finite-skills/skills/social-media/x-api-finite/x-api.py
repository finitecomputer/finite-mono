#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from typing import Any

API_BASE = "https://api.x.com/2"
DEFAULT_TWEET_FIELDS = ",".join(
    [
        "attachments",
        "author_id",
        "context_annotations",
        "conversation_id",
        "created_at",
        "entities",
        "in_reply_to_user_id",
        "lang",
        "possibly_sensitive",
        "public_metrics",
        "referenced_tweets",
        "source",
    ]
)
DEFAULT_USER_FIELDS = ",".join(
    [
        "created_at",
        "description",
        "location",
        "name",
        "profile_image_url",
        "protected",
        "public_metrics",
        "url",
        "username",
        "verified",
    ]
)
DEFAULT_MEDIA_FIELDS = ",".join(
    [
        "alt_text",
        "duration_ms",
        "height",
        "media_key",
        "preview_image_url",
        "public_metrics",
        "type",
        "url",
        "width",
    ]
)
DEFAULT_EXPANSIONS = ",".join(
    [
        "attachments.media_keys",
        "author_id",
        "in_reply_to_user_id",
        "referenced_tweets.id",
        "referenced_tweets.id.author_id",
    ]
)

STATUS_RE = re.compile(r"(?:https?://)?(?:www\.)?(?:x|twitter)\.com/[^/]+/status/(\d+)")


def bearer_token() -> str:
    token = os.environ.get("X_API_BEARER_TOKEN", "").strip()
    if not token:
        print("Error: X_API_BEARER_TOKEN not set", file=sys.stderr)
        raise SystemExit(1)
    return token


def api_get(path: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
    query = urllib.parse.urlencode(params or {}, doseq=True)
    url = f"{API_BASE}{path}"
    if query:
        url = f"{url}?{query}"

    request = urllib.request.Request(
        url,
        headers={
            "Authorization": f"Bearer {bearer_token()}",
            "Accept": "application/json",
            "User-Agent": "finite-x-api-finite/1.0",
        },
    )

    try:
        with urllib.request.urlopen(request) as response:
            return json.load(response)
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        print(f"X API error {exc.code}: {body}", file=sys.stderr)
        raise SystemExit(1)
    except urllib.error.URLError as exc:
        print(f"X API network error: {exc.reason}", file=sys.stderr)
        raise SystemExit(1)


def parse_status_id(value: str) -> str:
    raw = value.strip()
    if raw.isdigit():
        return raw
    match = STATUS_RE.search(raw)
    if match:
        return match.group(1)
    print(f"Could not parse tweet ID from: {value}", file=sys.stderr)
    raise SystemExit(1)


def build_include_maps(payload: dict[str, Any]) -> tuple[dict[str, dict[str, Any]], dict[str, dict[str, Any]]]:
    includes = payload.get("includes") or {}
    users_by_id = {entry["id"]: entry for entry in includes.get("users") or [] if entry.get("id")}
    media_by_key = {
        entry["media_key"]: entry for entry in includes.get("media") or [] if entry.get("media_key")
    }
    return users_by_id, media_by_key


def x_post_url(tweet: dict[str, Any], users_by_id: dict[str, dict[str, Any]]) -> str:
    author = users_by_id.get(tweet.get("author_id", ""), {})
    username = author.get("username")
    if username:
        return f"https://x.com/{username}/status/{tweet['id']}"
    return f"https://x.com/i/web/status/{tweet['id']}"


def format_metrics(metrics: dict[str, Any] | None) -> str:
    if not metrics:
        return "No public metrics"
    parts = []
    for key in [
        "like_count",
        "retweet_count",
        "reply_count",
        "quote_count",
        "bookmark_count",
        "impression_count",
    ]:
        if key in metrics:
            parts.append(f"{key.removesuffix('_count')}: {metrics[key]}")
    return ", ".join(parts) if parts else "No public metrics"


def format_media(tweet: dict[str, Any], media_by_key: dict[str, dict[str, Any]]) -> list[str]:
    attachments = tweet.get("attachments") or {}
    keys = attachments.get("media_keys") or []
    lines: list[str] = []
    for key in keys:
        media = media_by_key.get(key)
        if not media:
            continue
        line = f"- media `{key}`: {media.get('type', 'unknown')}"
        url = media.get("url") or media.get("preview_image_url")
        if url:
            line += f" ({url})"
        alt_text = media.get("alt_text")
        if alt_text:
            line += f" | alt: {alt_text}"
        lines.append(line)
    return lines


def print_tweets(payload: dict[str, Any], heading: str) -> None:
    tweets = payload.get("data") or []
    if isinstance(tweets, dict):
        tweets = [tweets]
    users_by_id, media_by_key = build_include_maps(payload)

    print(f"# {heading}\n")
    if not tweets:
        print("_No posts returned._")
        return

    for idx, tweet in enumerate(tweets, 1):
        author = users_by_id.get(tweet.get("author_id", ""), {})
        author_label = (
            f"@{author.get('username')} ({author.get('name')})"
            if author.get("username")
            else tweet.get("author_id", "unknown author")
        )
        print(f"## {idx}. {author_label}")
        print(f"- URL: {x_post_url(tweet, users_by_id)}")
        if tweet.get("created_at"):
            print(f"- Created: {tweet['created_at']}")
        if tweet.get("lang"):
            print(f"- Lang: {tweet['lang']}")
        if tweet.get("conversation_id"):
            print(f"- Conversation: {tweet['conversation_id']}")
        if tweet.get("source"):
            print(f"- Source: {tweet['source']}")
        print(f"- Metrics: {format_metrics(tweet.get('public_metrics'))}")
        print()
        print(tweet.get("text", "").strip())
        print()
        references = tweet.get("referenced_tweets") or []
        if references:
            print("Referenced posts:")
            for reference in references:
                print(f"- {reference.get('type', 'unknown')}: {reference.get('id', '')}")
            print()
        media_lines = format_media(tweet, media_by_key)
        if media_lines:
            print("Media:")
            for line in media_lines:
                print(line)
            print()


def cmd_lookup(args: argparse.Namespace) -> None:
    ids = [parse_status_id(value) for value in args.items]
    payload = api_get(
        "/tweets",
        {
            "ids": ",".join(ids),
            "expansions": DEFAULT_EXPANSIONS,
            "tweet.fields": DEFAULT_TWEET_FIELDS,
            "user.fields": DEFAULT_USER_FIELDS,
            "media.fields": DEFAULT_MEDIA_FIELDS,
        },
    )
    print_tweets(payload, "X Post Lookup")


def cmd_search(args: argparse.Namespace) -> None:
    requested = max(1, args.limit)
    api_limit = min(max(requested, 10), 100)
    payload = api_get(
        "/tweets/search/recent",
        {
            "query": args.query,
            "max_results": str(api_limit),
            "expansions": DEFAULT_EXPANSIONS,
            "tweet.fields": DEFAULT_TWEET_FIELDS,
            "user.fields": DEFAULT_USER_FIELDS,
            "media.fields": DEFAULT_MEDIA_FIELDS,
        },
    )
    if requested < api_limit and isinstance(payload.get("data"), list):
        payload["data"] = payload["data"][:requested]
    print_tweets(payload, f"X Recent Search: {args.query}")


def cmd_conversation(args: argparse.Namespace) -> None:
    tweet_id = parse_status_id(args.item)
    source_payload = api_get(
        f"/tweets/{tweet_id}",
        {
            "tweet.fields": "conversation_id",
        },
    )
    source_tweet = source_payload.get("data") or {}
    conversation_id = source_tweet.get("conversation_id") or tweet_id
    requested = max(1, args.limit)
    api_limit = min(max(requested, 10), 100)
    payload = api_get(
        "/tweets/search/recent",
        {
            "query": f"conversation_id:{conversation_id}",
            "max_results": str(api_limit),
            "expansions": DEFAULT_EXPANSIONS,
            "tweet.fields": DEFAULT_TWEET_FIELDS,
            "user.fields": DEFAULT_USER_FIELDS,
            "media.fields": DEFAULT_MEDIA_FIELDS,
        },
    )
    if requested < api_limit and isinstance(payload.get("data"), list):
        payload["data"] = payload["data"][:requested]
    print_tweets(payload, f"X Conversation: {conversation_id}")


def cmd_user(args: argparse.Namespace) -> None:
    handle = args.handle.lstrip("@")
    payload = api_get(
        f"/users/by/username/{urllib.parse.quote(handle)}",
        {
            "user.fields": DEFAULT_USER_FIELDS,
        },
    )
    user = payload.get("data") or {}
    print(f"# X User: @{handle}\n")
    if not user:
        print("_No user returned._")
        return
    print(f"- Name: {user.get('name', '')}")
    print(f"- Username: @{user.get('username', handle)}")
    if user.get("created_at"):
        print(f"- Created: {user['created_at']}")
    print(f"- Verified: {user.get('verified', False)}")
    if user.get("location"):
        print(f"- Location: {user['location']}")
    if user.get("url"):
        print(f"- URL: {user['url']}")
    metrics = user.get("public_metrics") or {}
    if metrics:
        print(
            "- Metrics: "
            + ", ".join(
                f"{label}: {metrics.get(key, 0)}"
                for key, label in [
                    ("followers_count", "followers"),
                    ("following_count", "following"),
                    ("tweet_count", "posts"),
                    ("listed_count", "listed"),
                ]
            )
        )
    description = user.get("description")
    if description:
        print()
        print(description)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Direct X API v2 helper for exact post and profile data.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    lookup = subparsers.add_parser("lookup", help="Look up one or more posts by URL or status ID.")
    lookup.add_argument("items", nargs="+", help="X/Twitter status URLs or numeric status IDs.")
    lookup.set_defaults(func=cmd_lookup)

    search = subparsers.add_parser("search", help="Run a recent search query.")
    search.add_argument("query", help="X recent-search query string.")
    search.add_argument("--limit", type=int, default=10, help="Number of posts to print.")
    search.set_defaults(func=cmd_search)

    conversation = subparsers.add_parser(
        "conversation", help="Fetch recent posts in the same conversation as a source post."
    )
    conversation.add_argument("item", help="X/Twitter status URL or numeric status ID.")
    conversation.add_argument("--limit", type=int, default=10, help="Number of posts to print.")
    conversation.set_defaults(func=cmd_conversation)

    user = subparsers.add_parser("user", help="Fetch a user profile by handle.")
    user.add_argument("handle", help="@handle or bare username.")
    user.set_defaults(func=cmd_user)

    return parser


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
