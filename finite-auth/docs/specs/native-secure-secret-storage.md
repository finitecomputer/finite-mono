# Native Secure Secret Storage Standard

## Purpose

Finite's first Frostr setup is a client-orchestrated 2-of-3 keyset. During
signup:

1. The client creates the Frostr keyset and group public key.
2. The client keeps the active User Client Share for normal signing.
3. The client writes the third share to native secure secret storage as the
   user's Cold Backup Share.
4. The client sends only the Server Share package to the server.
5. The client discards any temporary access to shares it does not own after
   packaging succeeds.

Normal user and default-agent signing uses the server share plus the active
user-client share. The Cold Backup Share is for recovery or rotation, not
routine signing and not unattended agent signing.

## Platform Standard

There is no single universal secure secret storage API. The standard is a small
Finite adapter over the platform-native store:

| Platform | Native store | Finite use |
| --- | --- | --- |
| Apple | Keychain Services, with access classes and optional user-presence ACLs | Store the Cold Backup Share package or a wrapping key for it. |
| Android | Android Keystore / KeyChain | Store a non-exportable wrapping key or protected share package, depending on host capabilities. |
| Windows | Credential Locker / Credential Manager, or DPAPI for protected blobs | Store the Cold Backup Share package or protect a local encrypted blob. |
| Linux desktop | Freedesktop Secret Service, implemented by providers such as GNOME Keyring, KWallet, or compatible password managers | Store the Cold Backup Share package as a secret item found by lookup attributes. |
| Rust host apps | `keyring` ecosystem | Use platform stores through one Rust adapter surface. |

Browser-only web clients do not have an equivalent native secret store. They
should use a native wrapper, platform passkey/credential integration, or an
explicit encrypted export/recovery flow rather than pretending browser storage
is the cold backup store.

## Rust `keyring` Guidance

For Rust desktop hosts, prefer the `keyring` ecosystem as the native secret
storage boundary:

- Use the `keyring` crate's simple mode when the app only needs
  platform-independent set/get/delete behavior for one secret per key.
- Use `keyring-core` plus specific store crates when the app must control the
  exact backend, access policy, or platform coverage.
- Do not put `keyring` inside `finite-auth-core`; keep it in host or adapter
  crates so server-only builds do not link desktop/mobile secret-store
  dependencies.
- Store only the serialized Frostr share package for the Native Secure Storage
  Share, or an encrypted package plus a platform-protected wrapping key. Never
  store a complete Nostr private key or reconstructed group secret.
- Use stable service/account names that can be regenerated from non-secret
  metadata. Example service: `finite-auth`. Example account:
  `frostr:<group-public-key-hex>:native-secure-storage-share`.
- Treat labels, account names, lookup attributes, and package references as
  non-secret metadata. Only the share package bytes are secret.

Many platform secret APIs are password-shaped. If the selected Rust store only
accepts text, encode the share package bytes with a reversible text encoding
before storage and authenticate the package format separately.

## Finite Metadata

`finite-auth` should persist metadata that identifies the cold backup share
without containing the share material:

```text
role: native-secure-storage
purpose: cold-backup
group_public_key: <nostr public key hex>
member_index: <frostr member index>
package_ref: <bounded host reference>
store_family: apple-keychain | android-keystore | windows-credential-locker | freedesktop-secret-service | rust-keyring | other
sync_policy: device-only | platform-sync | explicit-export | unknown
created_at: <timestamp>
last_verified_at: <timestamp, optional>
```

The `sync_policy` matters. If the active client share and Cold Backup Share are
on the same physical device and the native store is device-only, the cold share
helps recover from app storage loss but not from total device loss. If product
recovery must survive device loss, the host must choose platform sync, explicit
encrypted export, or another recovery path deliberately.

## Recovery Semantics

The Cold Backup Share may be unlocked by the user to recover or rotate the
keyset. Recovery should pair it with the server share, verify the group public
key, and then either restore a fresh active user-client share or rotate into a
new keyset. It should not silently become the normal client share.

## Source References

- Apple Keychain data protection: <https://support.apple.com/guide/security/keychain-data-protection-secb0694df1a/web>
- Android Keystore: <https://developer.android.com/privacy-and-security/keystore>
- Windows Credential Locker: <https://learn.microsoft.com/en-us/windows/apps/develop/security/credential-locker>
- Freedesktop Secret Service: <https://specifications.freedesktop.org/secret-service/>
- Rust `keyring`: <https://docs.rs/keyring>
