# NixOS SOPS Secret Sources

This directory is for encrypted SOPS source files only. Do not place plaintext
secret values, generated private keys, decrypted files, hashes, fingerprints, or
password-derived evidence here.

The operator workflow is documented in
[`docs/runs/nixos-sops-operator-flow.md`](../../../docs/runs/nixos-sops-operator-flow.md).
For the concise command reference, see
[`infra/secret/OPERATIONS.md`](../../secret/OPERATIONS.md).

The production host private age identities live only on the hosts:

- `finite-lat-1`: `/var/lib/sops-nix/finite-lat-1.agekey`
- `finite-lat-3`: `/var/lib/sops-nix/finite-lat-3.agekey`

Install them as `root:root 0600` under a `root:root 0700`
`/var/lib/sops-nix` directory. The corresponding public recipients can be
printed with:

```sh
age-keygen -y /var/lib/sops-nix/finite-lat-1.agekey
age-keygen -y /var/lib/sops-nix/finite-lat-3.agekey
```

Human and recovery private keys stay outside this repository under operator or
break-glass custody. The initial `.sops.yaml` may contain only human/recovery
recipients while encrypted sources are being staged, but those files are not
deployable until the host public recipients are added and
`just infra secrets updatekeys` has refreshed every encrypted file.

## Operator Key Setup

Run:

```sh
just infra secrets operator-key
```

The helper creates `~/.config/sops/age/keys.txt` if it is missing, or uses
`SOPS_AGE_KEY_FILE` when that environment variable is already set. It sets the
key directory to `0700`, the key file to `0600`, and prints only the public
`age1...` recipient. Add that public recipient to `.sops.yaml`; never commit or
share the private key file.

## Testing Decrypt Access

Run:

```sh
just infra secrets test-decrypt
```

The helper prints `true` when your current local age key can decrypt every
existing NixOS SOPS secret file. It prints `false` with a short next-step
message if you cannot decrypt existing files or if recipients are not configured
yet. It never prints plaintext secret values.

## Ingesting A Secret

Use `just infra secrets ingest` to encrypt plaintext from stdin into this
directory. The helper refuses interactive input, refuses path traversal, refuses
to overwrite by default, verifies local decrypt access, checks that the new
file's recipients match existing files in the same scope, and prints only the
encrypted target path plus a values-free Nix contract sketch.

Example for the metrics pilot:

```sh
ssh root@finite-lat-1 'sudo cat /etc/finite/metrics-remote-write.env' \
  | just infra secrets ingest \
      shared metrics-remote-write.env \
      --logical-name metrics-remote-write \
      --required-env-name FINITE_METRICS_REMOTE_WRITE_USERNAME \
      --required-env-name FINITE_METRICS_REMOTE_WRITE_PASSWORD \
      --consumer alloy.service \
      --restart-unit alloy.service
```

The command writes only the encrypted SOPS file, for example
`infra/nixos/secrets/shared/metrics-remote-write.env`. It does not add the Nix
contract entry for you; review the printed sketch and wire the relevant service
module in a separate commit.

If the helper says the current operator cannot decrypt existing files, add the
operator's public recipient to `.sops.yaml` and ask an existing operator to run
`just infra secrets updatekeys`. If it says recipient sets differ, review the
`.sops.yaml` change and run `just infra secrets updatekeys` before retrying.

## Updating Recipients

After adding or removing public recipients in `.sops.yaml`, refresh existing
encrypted files with:

```sh
just infra secrets updatekeys
```

The helper updates only SOPS JSON files under `infra/nixos/secrets`, skips
`.sops.yaml` and this README, runs `sops updatekeys --yes --input-type json`,
and prints only file paths. Use `--dry-run` to preview the file set:

```sh
just infra secrets updatekeys --dry-run
```

Removing a recipient and updating keys prevents that recipient from decrypting
future file revisions. It does not revoke plaintext someone already decrypted;
rotate the underlying secret when the trust boundary requires it.
