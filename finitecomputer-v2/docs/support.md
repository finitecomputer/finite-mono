# Support reports

Dashboard web chat handles `/support [description]` locally and opens a review
form. The header's **Contact support** action works without an Agent response;
chat errors and an offline/stale Runtime also show the direct contact address.
Replies go to the account's verified email. This version does not interpret
natural-language requests or add a Hermes/SimpleX command or an incoming email
bridge. Chat history, logs and attachments are never collected for reports.

Set `FINITE_SUPPORT_EMAIL` to the same single address on Dashboard and Core.
The first-party Nix configuration shares `support@finite.vip`; self-hosted
operators set their IT mailbox. Unset/invalid Dashboard configuration displays
“Contact your system administrator,” with no Finite fallback. Core independently
checks the reviewed recipient and reply address before accepting a report.

Core delivery additionally requires `FINITE_SUPPORT_MAILER=resend`,
`FINITE_SUPPORT_MAIL_FROM` (a verified sender address), and `RESEND_API_KEY`.
The first-party configuration uses `support@finite.chat` and the existing
send-only credential in `/etc/finite-saas/sites.env` (root-owned 0600).
Do not put mail credentials in Dashboard or Agent Runtimes. For development,
use `FINITE_SUPPORT_MAILER=dev` and `FINITE_SUPPORT_OUTBOX_DIR`; no provider
mail is sent. Bad/missing mail configuration disables submission without
preventing Core startup; users can still email the configured contact.

## Authority and persistence

The browser POSTs reviewed text, a request UUID, optional project id, and the
two reviewed addresses to Dashboard `/api/support`. Dashboard enforces JSON,
origin and body-size checks, then forwards its WorkOS session to Core
`POST /api/core/v1/me/support`. Core freshly verifies the account, derives
Reply-To from its verified email, checks project visibility and the configured
recipient, and permits five new reports per account per hour. Clients cannot
choose another recipient, impersonate a reply address, or submit on behalf of
an unrelated Project. Retrying the identical request returns the original
receipt; changing its contents with the same UUID is rejected.

The only new durable writer is the account support intake plus its Core email
worker. They own `support_requests`; no Chat, Device, Runner or Runtime
Management Pipe state is read or written by support delivery. The browser
reads a receipt, never other users' reports. The worker reads only due outbox
rows and sends their immutable reviewed body/address snapshot through
`finite-mail`, including Reply-To. Reports contain plaintext voluntarily sent
to support and belong in the existing Core database backup/recovery boundary.

Acceptance means **saved for email delivery**, not confirmed inbox delivery.
The worker leases one row for two minutes, retries failures after a minute,
and reuses the request id as the provider idempotency key. Restart retains
pending work. It stops retries after 23 hours, before Resend's documented
[24-hour idempotency window](https://resend.com/docs/dashboard/emails/idempotency-keys),
and retains the row as `failed` with an operator-attention log containing only
its reference. A crash after provider acceptance can leave an ambiguous result;
inspect provider delivery using the reference before any manual resend. No
claim of exactly-once inbox delivery is made. The local development outbox
records attempts and does not simulate provider deduplication.

## Deployment and rollback

Migration 0042 only adds the support table/indexes. Existing rows and old
Core/Dashboard readers remain unchanged. Deploy Core (migration, route and
worker) before Dashboard. A new Dashboard against old/disabled Core shows a
submission error and retains the reviewed payload for retry with the same key;
the direct email fallback stays visible. Old Dashboard against new Core keeps
its existing chat behavior. No Runtime image update or chat protocol change is
required. Runtime/account enrollment and chat readiness gates are unchanged.

Before enabling sending, verify the inbox is staffed, the configured sender is
verified, and a synthetic report reaches the correct inbox with working Reply-To.
Run the normal platform status checks before and after an authorized rollout.
Rollback may remove the UI and disable the support mailer; retain the additive
table and Core database backup. Pending reports pause until a capable worker
returns, and reports older than the retry boundary require operator attention.
Do not drop the table or resend ambiguous reports as part of rollback.
