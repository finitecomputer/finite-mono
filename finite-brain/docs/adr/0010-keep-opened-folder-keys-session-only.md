# Keep Opened Folder Keys Session-Only

Status: accepted

Opened Folder Keys are Session Folder Keys: trusted clients and Agent Runtimes
may cache encrypted Folder Key Grant envelopes and encrypted sync state, but
must not serialize raw Folder Keys into working-tree control files, browser
storage, logs, diagnostics, or other durable client state. A new process must
reopen grants through the acting Member Identity's signer and fail closed as
locked when that signer or a valid grant is unavailable; this removes
unnecessary secret copies while preserving offline restart when cached
encrypted grants and a local signer are available. Encrypted Recovery
Principal grants and Recovery Set artifacts remain intact; Session Folder Key
hardening must not remove or weaken a recovery path.
