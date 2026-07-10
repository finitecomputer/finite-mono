# Recoverability precedes operator-blindness

Status: principle accepted; Recovery Snapshot/key-backup design deferred as an
MVP TODO and open question.

Finite treats user data availability as the long-term first security invariant,
but a complete Recovery Snapshot, key-wrapping, and empty-target restore system
is not a launch gate for the first working SaaS slice. The first slice may rely
on provider durable storage and best-effort operational recovery for a trusted
cohort, provided we say plainly that full disaster recovery is not yet proven
and do not market zero-loss or operator-blind guarantees. The immediate product
gate is that ordinary restart/upgrade preserves mounted state and that users can
keep using the product.

Recovery Snapshot scope, Restic suitability, Recovery Authorities, key backup,
export, retention, and empty-target restore remain explicit TODOs. TEEs may
improve the normal privacy boundary, but Finite must not remove practical
operator escape routes or claim cryptographic operator-blindness until an
equivalent user-controlled recovery path is actually designed and exercised.
