# Backend Patterns On Finite

Use a backend only when the product needs server-side secrets, persistence,
webhooks, streaming, server rendering, or nontrivial API logic. Publish it as a
Finite Sites `kind = "app"` Project Output through `fsite`.

## Preferred Shape

Prefer one app process that serves the whole product:

- a Node or Bun server serving static files plus an API;
- a supported `uv` Python server with a static mount;
- a framework server whose production start command follows the Finite Sites
  runtime contract.

Finite Sites sets `PORT`; the server must listen on `0.0.0.0:$PORT`. Finite
Sites sets `DATA_DIR`; all live mutable state must live below that directory.

## Local Development

Examples:

```sh
PORT=8000 DATA_DIR="$PWD/.local-data" uv run uvicorn app:app --host 0.0.0.0 --port 8000
```

```sh
PORT=3000 DATA_DIR="$PWD/.local-data" node server.js
```

Install dependencies in user space or the project:

```sh
uv add fastapi uvicorn
npm install express
```

Test with a disposable local `DATA_DIR`. Do not seed tests into production
state.

## App Output

Declare the committed runtime bundle and start command:

```toml
[project]
slug = "project-name"

[outputs.web]
kind = "app"
site_name = "project-name"
branch = "main"
path = "app"
start = "uv run uvicorn app:app --host 0.0.0.0 --port $PORT"
```

Use `fsite describe workflow publish-stateful-app --output json` before
finalizing the configuration. Then validate with `fsite project init
--config finite.toml --dry-run --output json`, commit the app payload, and push
the Deploy Branch.

## Persistence

- Write live mutable state only under `DATA_DIR`.
- Treat the committed app directory as versioned, immutable deploy input.
- Do not overwrite or migrate user data destructively during startup.
- Make migrations replay-safe and preserve an escape path when they fail.
- A durable volume is not a backup; do not claim recovery until the same
  Recovery Set has restored onto an empty target.

## Secrets

- Keep credentials server-side and outside the Project Repository.
- Never ship API keys in client-side JavaScript.
- Do not commit `.env*`, `.finite/`, private keys, tokens, or local test data.

## LLM And Media Features

If the app uses an inference, search, document, or media service, read
`20-llm-api.md` and route credentialed calls through the backend.

## Validation

Before pushing:

- verify the server listens on `0.0.0.0:$PORT`;
- verify reads and writes remain under `DATA_DIR`;
- verify the frontend can reach the API;
- exercise one real request path end to end;
- restart against the same `DATA_DIR` and prove state remains accessible;
- inspect the resulting private Project Output before changing sharing.
