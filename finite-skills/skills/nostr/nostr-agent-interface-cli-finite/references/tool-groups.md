# Tool Groups

This grouped catalog reflects the current public skill release. If the upstream CLI adds or changes tools, `nostr-agent-interface cli list-tools --json` and `nostr-agent-interface cli <toolName> --help` are authoritative.

Prefer reads before writes. Use NIP-19 normalization before key-dependent mutations.

## Reading And Querying

- `getProfile`
- `getKind1Notes`
- `getLongFormNotes`
- `getReceivedZaps`
- `getSentZaps`
- `getAllZaps`
- `queryEvents`
- `getContactList`
- `getFollowing`
- `getRelayList`

## Identity And Profile

- `createKeypair`
- `createProfile`
- `updateProfile`

## Notes And Events

- `createNote`
- `signNote`
- `publishNote`
- `postNote`
- `createNostrEvent`
- `signNostrEvent`
- `publishNostrEvent`

## Social And Relay Management

- `setRelayList`
- `follow`
- `unfollow`
- `reactToEvent`
- `repostEvent`
- `deleteEvent`
- `replyToEvent`

## Messaging

- `encryptNip04`
- `decryptNip04`
- `sendDmNip04`
- `getDmConversationNip04`
- `encryptNip44`
- `decryptNip44`
- `sendDmNip44`
- `decryptDmNip44`
- `getDmInboxNip44`

## Anonymous Actions

- `sendAnonymousZap`
- `postAnonymousNote`

## NIP-19 Utilities

- `convertNip19`
- `analyzeNip19`

## Blossom Storage

- `getBlossomServers`
- `setBlossomServers`
- `getBlossomUrl`
- `uploadBlob`
- `downloadBlob`
- `listBlobs`
- `deleteBlob`
- `mirrorBlob`
