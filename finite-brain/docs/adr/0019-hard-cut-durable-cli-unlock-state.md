# Hard-Cut Durable CLI Unlock State

Status: accepted

`fbrain` operations reopen the encrypted Folder Key Grants they need into
process-local Session Folder Keys and discard those keys when the operation
ends. The separate `fbrain unlock` command, persisted `unlockedFolders`, and
the sync-unlock-sync workflow are a hard cut because a short-lived CLI process
cannot truthfully remain unlocked without persisting raw keys; the long-running
browser Product Client may still expose Session Lock and explicit unlock.
