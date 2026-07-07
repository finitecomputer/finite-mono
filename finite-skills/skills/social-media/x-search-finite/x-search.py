#!/usr/bin/env python3
"""
X/Twitter search and analysis tool powered by Grok + x_search.
Adapted from OpenUniverse (github.com/AnthonyRonning/openuniverse).

Commands:
  search <query> [--limit N]               Search tweets on any topic
  topic <query> --sides "A|B" [--limit N]  Topic analysis with side classification
  account <handle> --topics "t1,t2,t3"     Account analysis across topics
  ask <handle> <question>                  Freeform question about an account

Environment:
  XAI_API_KEY  Required. xAI API key for Grok.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from datetime import datetime


def get_client():
    from xai_sdk import Client

    api_key = os.environ.get("XAI_API_KEY")
    if not api_key:
        print("Error: XAI_API_KEY not set", file=sys.stderr)
        raise SystemExit(1)
    return Client(api_key=api_key)


def grok_chat(prompt: str, model: str = "grok-4-1-fast", handles: list[str] | None = None) -> str:
    """Send a prompt to Grok with x_search enabled and return response text."""
    from xai_sdk.chat import user
    from xai_sdk.tools import x_search

    client = get_client()
    tool = x_search(allowed_x_handles=handles) if handles else x_search()
    chat = client.chat.create(model=model, tools=[tool])
    chat.append(user(prompt))
    response = chat.sample()
    return response.content if hasattr(response, "content") else str(response)


def extract_json(text: str) -> dict | None:
    """Try to extract JSON from a Grok response that may be wrapped in markdown."""
    if "```json" in text:
        text = text.split("```json", 1)[1].split("```", 1)[0].strip()
    elif "```" in text:
        text = text.split("```", 1)[1].split("```", 1)[0].strip()
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return None


def now_label() -> str:
    return datetime.now().strftime("%Y-%m-%d %H:%M")


def cmd_search(args: argparse.Namespace) -> None:
    limit = args.limit or 10
    prompt = f"""Find the top {limit} most popular/viral tweets about: "{args.query}"

Return results as JSON:
{{
  "tweets": [
    {{
      "url": "https://x.com/username/status/...",
      "author": "@username",
      "text": "tweet text (first 280 chars)",
      "engagement": "likes/retweets summary"
    }}
  ]
}}

IMPORTANT: Every tweet MUST include its real x.com URL. Use x_search to find them. Focus on high engagement."""

    print(f"Searching X for: {args.query}", file=sys.stderr)
    raw = grok_chat(prompt)
    data = extract_json(raw)

    if data and "tweets" in data:
        print(f"# X Search: {args.query}\n")
        print(f"*{len(data['tweets'])} results | {now_label()}*\n")
        for i, tweet in enumerate(data["tweets"], 1):
            print(f"**{i}. {tweet.get('author', '???')}**")
            print(f"> {tweet.get('text', '')}")
            print(f"Engagement: {tweet.get('engagement', '')} | [Link]({tweet.get('url', '')})\n")
        return

    print(f"# X Search: {args.query}\n")
    print(raw)


def cmd_topic(args: argparse.Namespace) -> None:
    sides = args.sides.split("|")
    if len(sides) != 2:
        print("Error: --sides must be 'SideA|SideB'", file=sys.stderr)
        raise SystemExit(1)

    side_a, side_b = sides[0].strip(), sides[1].strip()
    limit = args.limit or 10

    search_prompt = f"""Find the top {limit} most popular/viral tweets about: "{args.query}"

Return as JSON:
{{
  "tweets": [
    {{
      "url": "https://x.com/username/status/...",
      "author": "@username",
      "text": "full tweet text",
      "engagement": "likes/retweets summary"
    }}
  ]
}}

IMPORTANT: Every tweet MUST include its real x.com URL. Use x_search. Focus on high-engagement tweets that express opinions."""

    print(f"Finding tweets about: {args.query}", file=sys.stderr)
    raw = grok_chat(search_prompt)
    tweets_data = extract_json(raw)

    if not tweets_data or "tweets" not in tweets_data:
        print(f"# Topic Analysis: {args.query}\n")
        print("Could not find tweets. Raw response:\n")
        print(raw)
        return

    tweets = tweets_data["tweets"]
    tweets_text = "\n\n".join(
        f"Tweet {i + 1} by {tweet.get('author', '?')}:\n{tweet.get('text', '')}"
        for i, tweet in enumerate(tweets)
    )

    classify_prompt = f"""Classify each tweet into one of two sides. Only use "neutral" if truly impossible to classify.

Side A = "{side_a}"
Side B = "{side_b}"

Tweets:
{tweets_text}

Return JSON:
{{
  "classifications": [
    {{"index": 1, "side": "a" or "b" or "neutral", "reason": "brief reason"}}
  ]
}}

Be decisive. Most tweets should clearly fall into side A or B."""

    print(f"Classifying into: {side_a} vs {side_b}", file=sys.stderr)
    raw2 = grok_chat(classify_prompt)
    class_data = extract_json(raw2)

    classifications: dict[int, dict] = {}
    if class_data and "classifications" in class_data:
        for classification in class_data["classifications"]:
            classifications[classification.get("index", 0)] = classification

    print(f"# Topic Analysis: {args.query}\n")
    print(f"**{side_a}** vs **{side_b}** | {len(tweets)} tweets | {now_label()}\n")

    counts = {"a": 0, "b": 0, "neutral": 0}
    for i, tweet in enumerate(tweets, 1):
        classification = classifications.get(i, {})
        side = classification.get("side", "?")
        reason = classification.get("reason", "")
        label = side_a if side == "a" else (side_b if side == "b" else "Neutral")
        counts[side] = counts.get(side, 0) + 1

        print(f"### {i}. {tweet.get('author', '?')} - *{label}*")
        print(f"> {tweet.get('text', '')}")
        print(f"*{reason}* | [Link]({tweet.get('url', '')})\n")

    print("---")
    print("## Summary")
    print(f"- **{side_a}**: {counts.get('a', 0)} tweets")
    print(f"- **{side_b}**: {counts.get('b', 0)} tweets")
    print(f"- **Neutral**: {counts.get('neutral', 0)} tweets")


def cmd_account(args: argparse.Namespace) -> None:
    handle = args.handle.lstrip("@")
    topics = [topic.strip() for topic in args.topics.split(",")]
    topic_list = "\n".join(f"- {topic}" for topic in topics)

    prompt = f"""Analyze the Twitter/X account @{handle} and determine their position on each topic:

{topic_list}

For each topic:
1. "active": true if they've discussed it, false otherwise
2. "position": brief description of their stance (max 1 sentence)
3. "examples": up to 3 REAL tweet URLs (https://x.com/username/status/ID) showing their position

IMPORTANT: Every claim must be backed by a real tweet URL. Use x_search to find relevant tweets.

Return JSON:
{{
  "account": "@{handle}",
  "topics": {{
    "Topic Name": {{
      "active": true/false,
      "position": "their stance",
      "examples": ["https://x.com/..."]
    }}
  }}
}}"""

    print(f"Analyzing @{handle} across {len(topics)} topics", file=sys.stderr)
    raw = grok_chat(prompt, handles=[handle])
    data = extract_json(raw)

    print(f"# Account Analysis: @{handle}\n")
    print(f"*{len(topics)} topics | {now_label()}*\n")

    if data and "topics" in data:
        for topic, info in data["topics"].items():
            active = "[active]" if info.get("active") else "[inactive]"
            print(f"## {active} {topic}")
            print(f"{info.get('position', 'No data')}\n")
            examples = info.get("examples", [])
            if examples:
                for url in examples:
                    print(f"- [{url}]({url})")
                print()
        return

    print(raw)


def cmd_ask(args: argparse.Namespace) -> None:
    handle = args.handle.lstrip("@")
    question = " ".join(args.question)

    prompt = f"""Answer this question about the Twitter/X account @{handle}:

"{question}"

CITATION RULES (mandatory):
- Use x_search to find relevant tweets
- EVERY claim must link to a specific tweet: [text](https://x.com/user/status/ID)
- Include at least one tweet URL per key point
- Format: markdown with inline links
- If you cannot find a source tweet, say so explicitly

Be concise and factual."""

    print(f"Asking about @{handle}: {question}", file=sys.stderr)
    raw = grok_chat(prompt, handles=[handle])

    print(f"# @{handle}: {question}\n")
    print(f"*{now_label()}*\n")
    print(raw)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="X/Twitter search and analysis via Grok")
    subparsers = parser.add_subparsers(dest="command")

    search_parser = subparsers.add_parser("search", help="Search tweets on a topic")
    search_parser.add_argument("query", nargs="+")
    search_parser.add_argument("--limit", type=int, default=10)

    topic_parser = subparsers.add_parser("topic", help="Topic analysis with side classification")
    topic_parser.add_argument("query", nargs="+")
    topic_parser.add_argument("--sides", required=True, help="'SideA|SideB'")
    topic_parser.add_argument("--limit", type=int, default=10)

    account_parser = subparsers.add_parser("account", help="Analyze account across topics")
    account_parser.add_argument("handle")
    account_parser.add_argument("--topics", required=True, help="Comma-separated topics")

    ask_parser = subparsers.add_parser("ask", help="Freeform question about an account")
    ask_parser.add_argument("handle")
    ask_parser.add_argument("question", nargs="+")

    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()

    if not args.command:
        parser.print_help()
        return 1

    if hasattr(args, "query"):
        args.query = " ".join(args.query)

    {
        "search": cmd_search,
        "topic": cmd_topic,
        "account": cmd_account,
        "ask": cmd_ask,
    }[args.command](args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
