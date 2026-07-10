# Enforce A Private Vault Working Tree Boundary

Status: accepted

`fbrain` creates and validates each explicit Vault Working Tree root and its
FiniteBrain-owned control state as private to the controlling OS account,
fails closed on insecure permissions or managed-path symlink escapes, and
offers an explicit repair path for existing trees. This boundary is deliberately
narrow: FiniteBrain does not recursively change arbitrary user content, and
the protection is filesystem isolation rather than a claim of application-level
encryption against the Trusted Device.
