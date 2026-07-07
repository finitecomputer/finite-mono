# Backend Patterns on Finite

Use a backend when the site needs server-side secrets, persistence, webhooks, streaming, or nontrivial API logic.

## Preferred Shape

Prefer one published app process that serves the whole product:

- Next.js app server
- Express app serving static files + API
- FastAPI app plus static mount
- Vite dev server during iteration, then a single production server if needed

The important platform rule is simple: publish one durable process on one port and let `finitec publish` remember how to start it.

## Starting a Backend

Examples:

```bash
uv run uvicorn app:app --host 0.0.0.0 --port 8000
```

```bash
node server.js
```

```bash
npm run dev -- --host 0.0.0.0 --port 3000
```

If dependencies are missing, install them in user space:

```bash
uv add fastapi uvicorn
npm install express
```

## Publishing a Backend-Backed App

```bash
finitec publish expose \
  --hostname HOSTNAME \
  --port 8000 \
  --run "uv run uvicorn app:app --host 0.0.0.0 --port 8000" \
  --cwd /home/node/workspace/project-name \
  --mode self
```

For a Node app:

```bash
finitec publish expose \
  --hostname HOSTNAME \
  --port 3000 \
  --run "node server.js" \
  --cwd /home/node/workspace/project-name \
  --mode self
```

## Persistence

- Durable app state lives in the machine home volume.
- Put project data under the project directory or another clear path under `/home/node`.
- Do not assume host-level shared storage.

## Secrets

- Shared keys and machine-specific secrets usually land in the Hermes environment for the runtime.
- Keep secrets on the server side.
- Never ship API keys in client-side JavaScript.

## LLM / Media Features

If the website uses OpenRouter, Firecrawl, Perplexity, ElevenLabs, or similar services, read `20-llm-api.md` and route those calls through the backend.

## Validation

Before publishing:

- verify the backend listens on the expected port
- verify the frontend can actually talk to it
- verify one real request path end to end
- publish privately first
