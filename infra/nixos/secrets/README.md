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
