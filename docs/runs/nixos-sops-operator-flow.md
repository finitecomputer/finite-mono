# NixOS SOPS Operator Flow

Status: ACTIVE MIGRATION SUPPORT

This document describes how Finite operators add encrypted NixOS secret source
files and how new operators become SOPS recipients. It is values-free: do not
paste plaintext secrets, derived hashes, fingerprints, or private keys into
this document, commits, issues, chat, or logs.

Concise command reference:
[`infra/nixos/secrets/OPERATIONS.md`](../../infra/nixos/secrets/OPERATIONS.md).

## Model

- Public `age1...` recipients are committed in
  `infra/nixos/secrets/.sops.yaml`.
- Private age keys are never committed. Each operator keeps their own local
  private key.
- Host private age keys live only on their hosts under `/var/lib/sops-nix`.
- A SOPS file is decryptable by every recipient listed in that file metadata.
- During bootstrap, files may be staged for human/recovery recipients before
  host recipients exist. They are not deployable until host recipients are added
  and `just nixos nixos-sops-updatekeys` is run.
- Updating `.sops.yaml` affects new files. Existing files must be rekeyed with
  `just nixos nixos-sops-updatekeys`.

## Become An Operator Recipient

Run:

```sh
just nixos nixos-sops-operator-key
```

The helper creates the local private key if missing, fixes permissions, and
prints only the public recipient:

```text
operator age key: created
key file: /Users/alex/.config/sops/age/keys.txt
key file mode: 0600
key directory mode: 0700
export SOPS_AGE_KEY_FILE=/Users/alex/.config/sops/age/keys.txt
public recipient: age1...
Add only the public recipient to infra/nixos/secrets/.sops.yaml.
```

Commit only the `age1...` public recipient in `.sops.yaml`. Do not commit the
private key file.

## Add A New Operator

1. The new operator runs `just nixos nixos-sops-operator-key`.
2. They add only their public `age1...` recipient to
   `infra/nixos/secrets/.sops.yaml`.
3. An existing operator who can decrypt current files runs:

```sh
just nixos nixos-sops-updatekeys
```

4. Commit `.sops.yaml` and the rekeyed SOPS files together.

The new operator can decrypt existing files only after step 3. The person
running `nixos-sops-updatekeys` must already be able to decrypt the existing
files.

## Test Operator Access

Run:

```sh
just nixos test-sops-decrypt
```

The helper prints `true` when the current local age key can decrypt every
existing NixOS SOPS secret file. It prints `false` and a short next-step message
when the operator cannot decrypt existing files, or when SOPS recipients have
not been configured yet. It captures decrypted output and never prints
plaintext.

## Add A Secret

Make sure your private key is the one SOPS should use:

```sh
export SOPS_AGE_KEY_FILE="$HOME/.config/sops/age/keys.txt"
```

Then pipe the plaintext into the ingestion helper. Example for the metrics
pilot:

```sh
ssh root@finite-lat-1 'sudo cat /etc/finite/metrics-remote-write.env' \
  | just nixos nixos-sops-ingest \
      shared metrics-remote-write.env \
      --logical-name metrics-remote-write \
      --required-env-name FINITE_METRICS_REMOTE_WRITE_USERNAME \
      --required-env-name FINITE_METRICS_REMOTE_WRITE_PASSWORD \
      --consumer alloy.service \
      --restart-unit alloy.service
```

The helper:

- refuses interactive input;
- refuses path traversal;
- refuses to overwrite unless `--force` is passed;
- refuses to proceed if the current operator cannot decrypt an existing SOPS
  file;
- encrypts from stdin without writing plaintext to a temp file;
- verifies the new file has the same recipient set as existing SOPS files in
  the same top-level scope;
- verifies the encrypted result is decryptable by the current operator before
  writing it;
- prints only the encrypted target path and a values-free Nix contract sketch.

If either decrypt verification fails, the operator is not a current usable
recipient for the SOPS set. Add their public recipient to `.sops.yaml`, have an
existing operator run `just nixos nixos-sops-updatekeys`, then retry. If the
recipient set check fails, `.sops.yaml` and the encrypted files disagree; run
`just nixos nixos-sops-updatekeys` after reviewing the `.sops.yaml` change,
then retry the ingest.

## Update Recipients

After changing `.sops.yaml`, preview affected files:

```sh
just nixos nixos-sops-updatekeys --dry-run
```

Then update metadata:

```sh
just nixos nixos-sops-updatekeys
```

This changes SOPS recipient metadata. It does not change the secret values.

## Safety Boundaries

The helpers make common mistakes harder, but they are not a substitute for
review:

- They do not know who is allowed to be a Finite operator. Code review of
  `.sops.yaml` is the canonical membership approval.
- `nixos-sops-ingest` proves that the current operator can decrypt existing
  files and the newly encrypted file, and that the new file's recipient metadata
  matches existing files in the same scope. It cannot prove every teammate has
  their private key installed until they run a decrypt themselves.
- `nixos-sops-updatekeys` can add or remove future decrypt access, but it
  cannot revoke plaintext someone already decrypted. Rotate underlying secrets
  when offboarding or trust boundaries require it.
- Production rollout still requires the migration runbook gates:
  finite-lat-2 build/eval proof, `scripts/finite-status` before rollout,
  service verification, and `scripts/finite-status` after rollout.
