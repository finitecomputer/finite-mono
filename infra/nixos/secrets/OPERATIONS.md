# NixOS SOPS Secret Operations

This is the operator command reference for NixOS SOPS-managed secret source
files. Do not paste plaintext secrets, decrypted values, hashes, fingerprints,
or private keys into docs, commits, issues, chat, or logs.

Use the root `just` wrappers from the repository root. The same recipes also
exist under the modular NixOS namespace, for example
`just nixos nixos-sops-ingest`, but the root names remain the stable operator
interface.

## Generate An Operator Recipient

Run:

```sh
just nixos-sops-operator-key
```

Commit or send only the printed `age1...` public recipient. Never commit or
share `~/.config/sops/age/keys.txt` or any private key material.

## Add A New Operator Recipient

1. The new operator runs:

   ```sh
   just nixos-sops-operator-key
   ```

2. Add only their printed `age1...` public recipient to
   `infra/nixos/secrets/.sops.yaml`.

3. An existing operator who can decrypt current files runs:

   ```sh
   just nixos-sops-updatekeys --dry-run
   just nixos-sops-updatekeys
   ```

4. Commit `.sops.yaml` and the rekeyed encrypted files together.

The new operator cannot decrypt existing files until step 3 is complete.

## Test Decrypt Access

Run:

```sh
just test-sops-decrypt
```

Expected success starts with:

```text
true
```

The helper captures decrypted output and never prints plaintext. If it prints
`false`, follow the next-step message: usually add the operator public recipient
to `.sops.yaml`, then ask an existing operator to run
`just nixos-sops-updatekeys`.

## Add A New Secret Source

Pipe plaintext directly into the ingest helper. Do not write plaintext to a
repo file.

Example:

```sh
some-command-that-prints-secret \
  | just nixos-sops-ingest \
      shared some-secret.env \
      --logical-name some-secret \
      --required-env-name SOME_ENV_NAME \
      --consumer some.service
```

For host-specific secrets, use the host scope instead of `shared`:

```sh
some-command-that-prints-secret \
  | just nixos-sops-ingest finite-lat-3 runner.env \
      --logical-name runner-env \
      --required-env-name FC_CORE_RUNNER_API_TOKEN \
      --consumer finite-saas-runner.service
```

The helper refuses interactive input, refuses path traversal, refuses overwrite
without `--force`, verifies decrypt access, and prints a values-free Nix
contract sketch. It does not add the `finite.secrets.files` entry for you.

## Update Recipient Metadata

After changing `.sops.yaml`, preview and apply recipient metadata updates:

```sh
just nixos-sops-updatekeys --dry-run
just nixos-sops-updatekeys
```

This changes who can decrypt future encrypted file revisions. It does not
change secret values, and it cannot revoke plaintext someone already decrypted.
Rotate underlying secrets when offboarding or changing trust boundaries.

## Decrypt For A Smoke Test

Prefer:

```sh
just test-sops-decrypt
```

If you must test one file with raw SOPS, redirect output away from the terminal:

```sh
sops decrypt infra/nixos/secrets/shared/metrics-remote-write.env >/dev/null
```

Do not run raw `sops decrypt` without redirecting output; it prints plaintext.

## Make A Secret Deployable

An encrypted source file is not used by NixOS until all of the following are
true:

- the relevant host public recipients are in `.sops.yaml`;
- `just nixos-sops-updatekeys` has refreshed the encrypted file metadata;
- a `finite.secrets.files.<name>` entry points at the encrypted file;
- the affected host closure has been evaluated and rolled out.

Current bootstrap staging may intentionally encrypt files to human/recovery
recipients first. Those files are reviewable and decryptable by operators, but
not deployable by hosts until host recipients are added and updatekeys is run.
