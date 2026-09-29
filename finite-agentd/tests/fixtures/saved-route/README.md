# Saved-route classifier fixtures

Shared by the Rust classifier (`finite-agentd/src/inference.rs`) and the Python port in
`finite_inference_helper.py`. Both must return `expected` for every file here.

Each file is one case:

- `model`: the raw value of `model` in Hermes `config.yaml`. It may be any JSON value.
- `fp_base_url`: the configured Finite Private base URL for this case, or `null` when none is
  configured. The product and retired URLs are recognized either way.
- `expected`: `finite_private`, `openrouter`, `openai_codex`, or `other`.
- `description`: what the case covers.

`model.provider` is trimmed and lowercased before it is matched, as Hermes does before it resolves
a provider. `model.base_url` is compared with scheme and host case-insensitive, a trailing `/` on
the path ignored, and everything else exact.
