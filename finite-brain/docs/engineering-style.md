# Engineering Style

FiniteBrain follows the full Finite engineering style in
[`finitechat/docs/engineering-style.md`](../../finitechat/docs/engineering-style.md).
That document is the canonical statement of the invariant, assertion, test,
allocation, and performance rules originally adopted for the chat protocol.
Do not replace it with a shortened local subset.

## FiniteBrain Local Rule

- Prefer hard cuts over compatibility shadow paths. Do not keep duplicate
  old/new APIs, fallbacks, launch/test-only shims, or parallel implementations
  merely to preserve pre-release tests or harnesses. Rewrite tests and callers
  to the new shape unless the user explicitly asks for backwards compatibility
  for real shipped users.
