# VIP Email binding requires email token and NIP-98

Binding a Finite VIP Email to a Native Principal requires both a valid email challenge token for that Finite VIP Email and a NIP-98-authenticated request signed by the target Local Identity Key. The email token proves control of the email name, while NIP-98 proves control of the Nostr key that will receive the NIP-05 Name; this matches the existing Finite Sites native email-linking shape and avoids binding a name to a pubkey the requester cannot sign with.
