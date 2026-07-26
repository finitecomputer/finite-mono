# Encrypted client-store compatibility fixtures

`client-state-v9-candidate.sqlite3` is a synthetic SQLite store written by the
real Finite Chat V9 client-store implementation from PR 303 at commit
`4dd0c4caf403c87598c0242fdf6b9ed062502d48`.

SHA-256:
`7bce8bc0025d05f8052a4acb1d5ae82f2489785643ed2e977f548018553e7db4`.

The fixture uses the public test-only account-secret bytes `[43; 32]`, Device
ID `v9-candidate-device`, and an unfinished link fanout named
`v9-candidate-fanout`. It contains no user or production data.

Do not regenerate this fixture with the compatibility writer. Its purpose is
to prove that the expand release can open bytes produced by the V9 candidate,
while the accompanying test proves that its next ordinary save still writes
V8 for rollback compatibility.
