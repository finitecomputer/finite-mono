# Encrypted client-store compatibility fixtures

## V8 predecessor

`client-state-v8-predecessor.sqlite3` is a synthetic SQLite store written by
the real Finite Chat V8 client-store implementation at commit
`5276a77d63a1b5069d21f3c2f5289947728f2784`, the direct predecessor of the V9
snapshot writer.

SHA-256:
`a81d89cc2f22d93810dd513b6bfafd5a85eccad653c70ddcf53a8a457895fc57`.

The fixture uses the public test-only account-secret bytes `[42; 32]`, Device
ID `v8-predecessor-device`, and an unfinished link fanout named
`v8-predecessor-fanout`. It contains no user or production data.

Do not regenerate this fixture with the current writer. Its purpose is to make
the V9 reader open bytes produced by the predecessor implementation, including
the V8 absence of the V9 bootstrap-export field.

## V9 candidate

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
while its accompanying test proves that the expand release's next ordinary
save still writes V8 for rollback compatibility.
