#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import sys
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from typing import Any

GAMMA = "https://gamma-api.polymarket.com"
CLOB = "https://clob.polymarket.com"
DATA = "https://data-api.polymarket.com"


def get_json(url: str) -> dict[str, Any] | list[Any]:
    request = urllib.request.Request(url, headers={"User-Agent": "finite-polymarket-finite/1.0"})
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.loads(response.read().decode("utf-8"))


def parse_json_field(value: Any) -> Any:
    if isinstance(value, str):
        try:
            return json.loads(value)
        except (json.JSONDecodeError, TypeError):
            return value
    return value


def normalize_market(market: dict[str, Any]) -> dict[str, Any]:
    copy = dict(market)
    for key in ("outcomes", "outcomePrices", "clobTokenIds"):
        copy[key] = parse_json_field(copy.get(key))
    return copy


def fmt_pct(value: Any) -> str:
    try:
        return f"{float(value) * 100:.1f}%"
    except (TypeError, ValueError):
        return str(value)


def fmt_volume(value: Any) -> str:
    try:
        amount = float(value)
    except (TypeError, ValueError):
        return str(value)
    if amount >= 1_000_000:
        return f"${amount / 1_000_000:.1f}M"
    if amount >= 1_000:
        return f"${amount / 1_000:.1f}K"
    return f"${amount:.0f}"


def render_market(market: dict[str, Any], *, prefix: str = "") -> list[str]:
    market = normalize_market(market)
    lines = [f"{prefix}{market.get('question', '?')}"]
    outcomes = market.get("outcomes") or []
    prices = market.get("outcomePrices") or []
    if isinstance(outcomes, list) and isinstance(prices, list):
        formatted = []
        for index, price in enumerate(prices):
            label = outcomes[index] if index < len(outcomes) else f"Outcome {index + 1}"
            formatted.append(f"{label}: {fmt_pct(price)}")
        if formatted:
            lines.append(f"{prefix}  {' / '.join(formatted)}")
    lines.append(f"{prefix}  volume={fmt_volume(market.get('volume', 0))} active={market.get('active', '?')} closed={market.get('closed', '?')}")
    if market.get("slug"):
        lines.append(f"{prefix}  slug={market['slug']}")
    if market.get("conditionId"):
        lines.append(f"{prefix}  conditionId={market['conditionId']}")
    token_ids = market.get("clobTokenIds") or []
    if isinstance(token_ids, list) and token_ids:
        lines.append(f"{prefix}  token_ids={', '.join(token_ids)}")
    return lines


def cmd_search(args: argparse.Namespace) -> int:
    payload = get_json(f"{GAMMA}/public-search?q={urllib.parse.quote(args.query)}")
    events = payload.get("events") or []
    if args.json:
        print(json.dumps(payload, indent=2))
        return 0
    total = (payload.get("pagination") or {}).get("totalResults", len(events))
    print(f'Found {total} results for "{args.query}":\n')
    for event in events[: args.limit]:
        print(f"=== {event.get('title', '?')} ===")
        print(f"slug={event.get('slug', '')} volume={fmt_volume(event.get('volume', 0))}")
        for market in (event.get("markets") or [])[: args.market_limit]:
            print("\n".join(render_market(market, prefix="  ")))
        print("")
    return 0


def cmd_trending(args: argparse.Namespace) -> int:
    payload = get_json(
        f"{GAMMA}/events?limit={args.limit}&active=true&closed=false&order=volume&ascending=false"
    )
    if args.json:
        print(json.dumps(payload, indent=2))
        return 0
    for index, event in enumerate(payload, start=1):
        print(f"{index}. {event.get('title', '?')}")
        print(f"   slug={event.get('slug', '')} volume={fmt_volume(event.get('volume', 0))} markets={len(event.get('markets') or [])}")
        for market in (event.get("markets") or [])[: args.market_limit]:
            print("\n".join(render_market(market, prefix="   ")))
        print("")
    return 0


def cmd_event(args: argparse.Namespace) -> int:
    payload = get_json(f"{GAMMA}/events?slug={urllib.parse.quote(args.slug)}")
    if args.json:
        print(json.dumps(payload, indent=2))
        return 0
    if not payload:
        print(f"No event found for slug {args.slug}", file=sys.stderr)
        return 1
    event = payload[0]
    print(f"{event.get('title', '?')}")
    print(f"slug={event.get('slug', '')} volume={fmt_volume(event.get('volume', 0))} liquidity={fmt_volume(event.get('liquidity', 0))}")
    description = event.get("description")
    if description:
        print(f"\n{description}\n")
    for market in event.get("markets") or []:
        print("\n".join(render_market(market, prefix="  ")))
        print("")
    return 0


def cmd_market(args: argparse.Namespace) -> int:
    payload = get_json(f"{GAMMA}/markets?slug={urllib.parse.quote(args.slug)}")
    if args.json:
        print(json.dumps(payload, indent=2))
        return 0
    if not payload:
        print(f"No market found for slug {args.slug}", file=sys.stderr)
        return 1
    market = payload[0]
    print("\n".join(render_market(market)))
    description = market.get("description")
    if description:
        print(f"\n{description}")
    return 0


def cmd_price(args: argparse.Namespace) -> int:
    payload = get_json(f"{CLOB}/price?token_id={urllib.parse.quote(args.token_id)}&side={urllib.parse.quote(args.side)}")
    if args.json:
        print(json.dumps(payload, indent=2))
        return 0
    print(f"{args.token_id}")
    print(f"{args.side} price: {fmt_pct(payload.get('price'))}")
    return 0


def cmd_book(args: argparse.Namespace) -> int:
    payload = get_json(f"{CLOB}/book?token_id={urllib.parse.quote(args.token_id)}")
    if args.json:
        print(json.dumps(payload, indent=2))
        return 0
    print(f"Orderbook for {args.token_id}")
    print(f"last_trade={fmt_pct(payload.get('last_trade_price'))} tick_size={payload.get('tick_size', '?')}")
    print("\nBids:")
    for bid in (payload.get("bids") or [])[: args.limit]:
        print(f"  {fmt_pct(bid.get('price')):>7}  size={bid.get('size')}")
    print("\nAsks:")
    for ask in (payload.get("asks") or [])[: args.limit]:
        print(f"  {fmt_pct(ask.get('price')):>7}  size={ask.get('size')}")
    return 0


def cmd_history(args: argparse.Namespace) -> int:
    payload = get_json(
        f"{CLOB}/prices-history?market={urllib.parse.quote(args.condition_id)}&interval={urllib.parse.quote(args.interval)}&fidelity={args.fidelity}"
    )
    history = payload.get("history") or []
    if args.json:
        print(json.dumps(payload, indent=2))
        return 0
    if not history:
        print("No price history available.")
        return 0
    for point in history:
        timestamp = datetime.fromtimestamp(point["t"], tz=timezone.utc).strftime("%Y-%m-%d %H:%M")
        print(f"{timestamp}  {fmt_pct(point.get('p'))}")
    return 0


def cmd_trades(args: argparse.Namespace) -> int:
    url = f"{DATA}/trades?limit={args.limit}"
    if args.condition_id:
        url += f"&market={urllib.parse.quote(args.condition_id)}"
    payload = get_json(url)
    if args.json:
        print(json.dumps(payload, indent=2))
        return 0
    for trade in payload:
        timestamp = trade.get("timestamp", "")
        print(
            f"{trade.get('side', '?'):>4}  {fmt_pct(trade.get('price')):>7}  "
            f"size={trade.get('size')}  outcome={trade.get('outcome', '?')}  {timestamp}"
        )
        if trade.get("title"):
            print(f"      {trade['title']}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Polymarket public API helper")
    subparsers = parser.add_subparsers(dest="command", required=True)

    search = subparsers.add_parser("search", help="Search Polymarket events")
    search.add_argument("--query", required=True)
    search.add_argument("--limit", type=int, default=5)
    search.add_argument("--market-limit", type=int, default=3)
    search.add_argument("--json", action="store_true")
    search.set_defaults(func=cmd_search)

    trending = subparsers.add_parser("trending", help="List top active events by volume")
    trending.add_argument("--limit", type=int, default=10)
    trending.add_argument("--market-limit", type=int, default=3)
    trending.add_argument("--json", action="store_true")
    trending.set_defaults(func=cmd_trending)

    event = subparsers.add_parser("event", help="Fetch an event by slug")
    event.add_argument("--slug", required=True)
    event.add_argument("--json", action="store_true")
    event.set_defaults(func=cmd_event)

    market = subparsers.add_parser("market", help="Fetch a market by slug")
    market.add_argument("--slug", required=True)
    market.add_argument("--json", action="store_true")
    market.set_defaults(func=cmd_market)

    price = subparsers.add_parser("price", help="Fetch current price for a token")
    price.add_argument("--token-id", required=True)
    price.add_argument("--side", default="buy", choices=["buy", "sell"])
    price.add_argument("--json", action="store_true")
    price.set_defaults(func=cmd_price)

    book = subparsers.add_parser("book", help="Fetch orderbook for a token")
    book.add_argument("--token-id", required=True)
    book.add_argument("--limit", type=int, default=10)
    book.add_argument("--json", action="store_true")
    book.set_defaults(func=cmd_book)

    history = subparsers.add_parser("history", help="Fetch price history by condition ID")
    history.add_argument("--condition-id", required=True)
    history.add_argument("--interval", default="all", choices=["all", "1d", "1w", "1m", "3m", "6m", "1y"])
    history.add_argument("--fidelity", type=int, default=50)
    history.add_argument("--json", action="store_true")
    history.set_defaults(func=cmd_history)

    trades = subparsers.add_parser("trades", help="Fetch recent trades")
    trades.add_argument("--limit", type=int, default=10)
    trades.add_argument("--condition-id")
    trades.add_argument("--json", action="store_true")
    trades.set_defaults(func=cmd_trades)

    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
