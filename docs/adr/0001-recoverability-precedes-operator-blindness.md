# Recoverability precedes operator-blindness

User data availability is the first security invariant. A TEE or provider
persistent volume is not a backup. A recovery claim requires the complete
Recovery Set, independently held recovery authority, and a successful restore
onto an empty target.

Do not remove an operator recovery path or claim stronger operator-blindness
until an equivalent recovery path has been exercised. Routine restart and
upgrade must preserve mounted state and identity. Compute retirement preserves
recovery material; irreversible data purge requires separate authorization and
retention checks.

State the tested failure coverage and recovery limits honestly. Missing
recovery capabilities are tracked in [Linear](https://linear.app/finitecomputer/issue/FIN-62).
