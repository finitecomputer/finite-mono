# Use Server-Sent Brain Update Notifications

Status: accepted

FiniteBrain clients will receive content-free **Brain Update Notifications**
through one authenticated Server-Sent Events connection per active client
identity instead of polling or using bidirectional WebSockets. Notifications
name the affected Brain and distinguish `content_updated` from
`access_updated`; they are coalescible hints, never durable sync records or a
second source of truth. Clients reconcile through the existing authenticated,
sequence-based sync contract, and reconnect by comparing authoritative Brain
sequences rather than replaying missed notifications.

The browser listens only while its Brain session is active and reconciles only
the open Brain. Each hosted Agent Runtime supervises one identity-level Brain
sync process that listens once and manages every existing open Brain Working
Tree. Content-update bursts for one Brain are briefly coalesced, while access
updates are handled immediately. The current single-server SQLite deployment
uses an internal in-process broadcaster behind a notification interface; a
shared broker becomes required before FiniteBrain runs multiple server
instances.

Automatic reconciliation applies remote changes only where they do not
overwrite unsaved local work. Conflicting browser drafts or agent files are
preserved for explicit resolution while unrelated changes continue. On access
loss, the browser locks and clears the affected decrypted session state; a
hosted agent pauses sync but preserves its already-created persistent Working
Tree and unsynced edits, because revocation cannot recall plaintext already
received.
