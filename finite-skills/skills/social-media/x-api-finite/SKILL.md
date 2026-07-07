---
name: x-api-finite
description: Direct X API v2 access for exact tweet lookup, recent search, conversation inspection, and user profile fetches when the human pastes an x.com status URL or browser access is brittle.
---

# X API

Use the official X API v2 when you need exact post data instead of Grok search summaries.

Use this skill when:

- the human pasted an `x.com/.../status/...` or `twitter.com/.../status/...` URL
- you need the real post text, author, metrics, or media metadata
- browser access is failing because X blocks or rate-limits web scraping
- you want recent-search results from the actual X API, not LLM synthesis

## Setup

Requires `X_API_BEARER_TOKEN` in the Hermes environment.

Use the local helper directly:

```bash
python3 /profile-assets/hermes-local/managed-skills/social-media/x-api-finite/x-api.py lookup https://x.com/jack/status/20
python3 /profile-assets/hermes-local/managed-skills/social-media/x-api-finite/x-api.py search "from:jack nostr" --limit 5
python3 /profile-assets/hermes-local/managed-skills/social-media/x-api-finite/x-api.py user @jack
python3 /profile-assets/hermes-local/managed-skills/social-media/x-api-finite/x-api.py conversation https://x.com/jack/status/20 --limit 10
```

## Workflow

1. If the human pasted a specific X URL, start with `lookup`.
2. If they want the surrounding replies, use `conversation`.
3. If they want broader discovery, use `search`.
4. If they want exact account metadata, use `user`.
5. Prefer this skill over browser-based X inspection unless the task specifically needs rendered UI.

## Notes

- `lookup` accepts full URLs, bare status IDs, or multiple inputs at once.
- `search` uses the X v2 recent-search endpoint and returns exact post links plus metrics.
- `conversation` is a convenience wrapper over recent search using `conversation_id:<tweet_id>`.
- This skill is read-only. It does not post, like, or follow.
