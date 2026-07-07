# Engineering Style

## Local Rules

- Prefer hard cuts over compatibility shadow paths. Do not keep duplicate
  old/new APIs, fallbacks, launch/test-only shims, or parallel implementations
  merely to preserve pre-release tests or harnesses. Rewrite tests and callers
  to the new shape unless the user explicitly asks for backwards compatibility
  for real shipped users.
