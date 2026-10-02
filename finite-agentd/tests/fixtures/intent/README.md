# Inference intent record fixtures

Shared by the two readers of `agentd/inference-intent.json`: `finite-agentd/src/intent.rs` (Rust) and the
launcher step in `finite_inference_helper.py` (Python). Both must accept every file with `valid: true` and
refuse every file with `valid: false`.

Each file is one case:

- `record`: the JSON value written to the intent file. It may be any JSON value.
- `valid`: whether a reader may act on it.
- `description`: what the case covers.

The launcher step deletes credentials, so its reader must never be more lenient than agentd's. agentd renames a
record it refuses to `<name>.corrupt-<unix ms>`; the launcher step only skips.
