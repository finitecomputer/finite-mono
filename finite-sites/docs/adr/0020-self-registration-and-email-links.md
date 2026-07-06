# Self-Registration And Email Links

Finite Sites uses native npub auth as the default publishing identity. A new
agent should not need an operator allowlist round trip before it can create a
Project Repository for a paying Finite user.

Decision:

- `fsite auth register [--output json]` is the explicit bootstrap command.
  It signs with the local User Key and creates or replays a self-registered
  publish grant up to the v0 Publishing Limit.
- Email is optional. Npub-primary users can create Projects without ever
  sharing an email address.
- `fsite auth link-email EMAIL` sends a normal email token and stores a local
  pending-link marker containing only the target native pubkey.
- `fsite auth redeem EMAIL TOKEN` uses the native User Key when a matching
  pending-link marker exists; otherwise it preserves the email-only External
  Principal fallback.
- `fsite auth redeem EMAIL TOKEN --link-native` lets an email invite token be
  linked to the local native Principal directly, without requesting a second
  token.
- A verified Email Link maps one email address to one Native Principal. Future
  Project Grants to that email resolve to the linked native Principal.
- Linking migrates existing active Project Collaborator grants from the
  External Principal to the Native Principal and revokes old email-scoped Git
  Credentials. Replay is a no-op.
- Operator grants remain useful for manual override and revocation, but they
  are no longer the default onboarding path.

Considered options:

- Keep operator allowlisting as the only publishing bootstrap. This clamps
  abuse, but it blocks the agent-led growth loop and forces Paul/operator
  interaction for every new bot.
- Auto-register inside `project init`. That is convenient but too magical for
  agent UX; when authorization fails, the CLI should name the missing auth
  primitive and let the agent run it explicitly.
- Make email required. This matches current customer bot accounts, but it
  fights the long-term npub-primary model.
- Infer email ownership from context. Rejected because Principal Links must be
  explicit and verified.

Consequences:

- Abuse control moves from pre-approval to revocation, output limits, and
  future paid/Core grants.
- Agents can learn the whole bootstrap from CLI help:
  `fsite describe workflow register-and-publish --output json`.
- Email-only auth remains available for external users and non-Finite agents.
