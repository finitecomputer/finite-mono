---
name: x-search-finite
description: Search and analyze X/Twitter using Grok's x_search. Topic search, account analysis, side-by-side debate classification, and freeform questions.
---

# X Search

Search and analyze X/Twitter content using Grok + x_search. Adapted from [OpenUniverse](https://github.com/AnthonyRonning/openuniverse) and pulled from [paul-and-waffle](https://github.com/waffledog-bot/paul-and-waffle/tree/main/skills/x-search).

## Setup

Requires `XAI_API_KEY` and the `xai-sdk` Python package. The current hosted
runtime does not promise that package; if its import fails, report the runtime
packaging gap instead of claiming this skill is ready.

## Commands

Use the local helper directly:

```bash
SKILL_ROOT="${FINITECHAT_HOME:-/data/agent}/managed-skills/finite/current/social-media/x-search-finite"
python3 "$SKILL_ROOT/x-search.py" search "bitcoin etf" --limit 5
python3 "$SKILL_ROOT/x-search.py" topic "AI regulation" --sides "Pro-regulation|Anti-regulation" --limit 10
python3 "$SKILL_ROOT/x-search.py" account @elonmusk --topics "AI,Bitcoin,Free speech,Mars"
python3 "$SKILL_ROOT/x-search.py" ask @jack "What does he think about Nostr?"
```

## Output

All output is markdown. Pipe to a file to save reports:

```bash
python3 "$SKILL_ROOT/x-search.py" account @jack --topics "Bitcoin,Nostr,Bluesky" > reports/jack-analysis.md
```

## How It Works

This skill uses Grok with the built-in `x_search` tool, so it only needs the xAI key and does not require separate X API credentials.

- Model: defaults to `grok-4-1-fast`
- Only Grok 4+ supports `x_search`
- Output is citation-heavy markdown intended for saving or synthesis

## Features

- Topic search with engagement summaries
- Two-sided topic classification
- Account analysis across configurable topics
- Freeform account questions
- Markdown reports with tweet links

## Notes

- Prefer `search` or `ask` for broader investigations; `account` is best when you already know the handle you care about.
- Ask for high-engagement or viral tweets if you want better signal.
- The `topic` command depends on Grok returning parseable JSON between phases, so `ask` is often more reliable for complex investigations.

## Sources

- Upstream skill: https://github.com/waffledog-bot/paul-and-waffle/tree/main/skills/x-search
- OpenUniverse reference: https://github.com/AnthonyRonning/openuniverse
