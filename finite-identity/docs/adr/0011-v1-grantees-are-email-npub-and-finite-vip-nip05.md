# V1 identity targets are typed mailbox, NIP-05, and npub inputs

In v1, product CLIs accept identity targets through distinct `--email`,
`--nip05`, and `--npub` inputs. `--email` means a deliverable Mailbox Address;
`--nip05` means a resolution name; `--npub` means the native key directly.
Their similar string shapes never make them interchangeable.

A Finite NIP-05 Name resolves through the Identity Authority to exactly one
native `npub`. A Managed Agent NIP-05 is explicitly classified as
non-deliverable, so passing it to `--email` fails before any challenge or
product invite is created and points the caller to `--nip05`.

Third-Party NIP-05 resolution remains future work. Third-party Mailbox
Addresses remain valid invitation targets through `--email`.
