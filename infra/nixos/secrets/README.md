# NixOS SOPS Secret Sources

This directory is for encrypted SOPS source files only. Do not place plaintext
secret values, generated private keys, decrypted files, hashes, fingerprints, or
password-derived evidence here.

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
break-glass custody. After those public recipients and the host public
recipients are known, add `infra/nixos/secrets/.sops.yaml` creation rules before
encrypting any production material.

## Ingesting A Secret

Use `just nixos-sops-ingest` to encrypt plaintext from stdin into this
directory. The helper refuses interactive input, refuses path traversal, refuses
to overwrite by default, and prints only the encrypted target path plus a
values-free Nix contract sketch.

Example for the metrics pilot:

```sh
ssh root@finite-lat-1 'sudo cat /etc/finite/metrics-remote-write.env' \
  | just nixos-sops-ingest \
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
