---
name: polymarket-finite
description: Query Polymarket prediction market data — search markets, inspect events, fetch prices, orderbooks, price history, and trades through the public APIs with a bundled helper script.
version: 1.1.0
author: Hermes Agent + Teknium
tags: [polymarket, prediction-markets, market-data, trading]
---

# Polymarket

Use the bundled helper script instead of hand-assembling curl requests.

No API key is needed.

Script path:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/polymarket-finite/scripts/polymarket.py --help
```

## Workflow

1. `search` when the user asks about a topic or event
2. `event` or `market` once you have a slug
3. `price`, `book`, `history`, or `trades` for deeper market inspection

## Commands

Search:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/polymarket-finite/scripts/polymarket.py search \
  --query "OpenAI funding" \
  --limit 5
```

Trending events:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/polymarket-finite/scripts/polymarket.py trending \
  --limit 10
```

Specific event:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/polymarket-finite/scripts/polymarket.py event \
  --slug "some-event-slug"
```

Specific market:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/polymarket-finite/scripts/polymarket.py market \
  --slug "some-market-slug"
```

Token price and orderbook:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/polymarket-finite/scripts/polymarket.py price \
  --token-id "TOKEN_ID"

python3 /profile-assets/hermes-local/managed-skills/research/polymarket-finite/scripts/polymarket.py book \
  --token-id "TOKEN_ID" \
  --limit 10
```

Price history and trades:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/polymarket-finite/scripts/polymarket.py history \
  --condition-id "0xCONDITION_ID" \
  --interval 1m \
  --fidelity 30

python3 /profile-assets/hermes-local/managed-skills/research/polymarket-finite/scripts/polymarket.py trades \
  --condition-id "0xCONDITION_ID" \
  --limit 10
```

Machine-readable output:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/polymarket-finite/scripts/polymarket.py search \
  --query "OpenAI funding" \
  --json
```

## Notes

- Gamma API is best for discovery and slugs.
- CLOB API is best for prices, orderbooks, and history.
- Data API is best for recent trades.
- Prices are probabilities: `0.65` means `65%`.
- Gamma returns `outcomes`, `outcomePrices`, and `clobTokenIds` as JSON strings inside JSON; the helper script normalizes those for you.
