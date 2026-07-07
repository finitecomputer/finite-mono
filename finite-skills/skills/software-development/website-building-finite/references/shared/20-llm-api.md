# LLM & External API Use on Finite

Websites on finite can use shared API keys, but the safe default is:

- keep keys server-side
- call them from a backend you control
- publish privately first

## Available Platform-Level Keys

Depending on the box baseline, these may already be configured in the runtime environment:

- `OPENROUTER_API_KEY`
- `FIRECRAWL_API_KEY`
- `PERPLEXITY_API_KEY`
- `ELEVENLABS_API_KEY`

Do not assume a key exists blindly. Check the environment or the machine config first.

## Preferred Architecture

- Frontend collects input and renders results.
- Backend calls the model or external API.
- Backend returns sanitized results to the frontend.

Do not put long-lived API secrets into browser JavaScript bundles.

## OpenRouter

Use OpenRouter for text and multimodal model calls from a backend.

Python example:

```python
import os
from openai import OpenAI

client = OpenAI(
    api_key=os.environ["OPENROUTER_API_KEY"],
    base_url="https://openrouter.ai/api/v1",
)

response = client.chat.completions.create(
    model="anthropic/claude-sonnet-4.6",
    messages=[{"role": "user", "content": "Summarize this page"}],
)
```

## Firecrawl

Use Firecrawl for crawling and extraction tasks that should not happen in the browser:

```python
import os
import requests

response = requests.post(
    "https://api.firecrawl.dev/v1/scrape",
    headers={"Authorization": f"Bearer {os.environ['FIRECRAWL_API_KEY']}"},
    json={"url": "https://example.com"},
    timeout=60,
)
response.raise_for_status()
data = response.json()
```

## Perplexity

Use Perplexity from the backend when the site needs grounded search or cited summaries. Prefer surfacing citations and URLs back to the user rather than hiding them.

## ElevenLabs

Use ElevenLabs from the backend for TTS or speech features. Avoid shipping the key client-side.

## Publish Guardrail

If a site exposes billable model-backed features or external-tool actions:

- explain the risk plainly before publishing
- default to `self` or `emails`
- require explicit `MAKE PUBLIC` before public release

This matters more for AI sites than for ordinary static sites.
