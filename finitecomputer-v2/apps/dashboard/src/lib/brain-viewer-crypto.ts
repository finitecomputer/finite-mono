/**
 * Browser-side crypto for the brain:// live viewer (plan Phase 2, grill #4).
 *
 * The ephemeral viewer key lives in tab memory only — nothing here may
 * write durable storage. The Brain server stays blind: it relays NIP-44
 * wrapped Folder Keys and AES-256-GCM record ciphertext it cannot open.
 *
 * Mirrors of the Rust shapes:
 * - wrap: finite-nostr nip44 (Rust ↔ nostr-tools interop pinned by a
 *   fixed vector test in finite-nostr/src/nip44.rs)
 * - record envelope: finite-brain-core `finite-folder-object-v1`
 *   (AES-256-GCM, 12-byte nonce, canonical-JSON AAD)
 */

import { bytesToHex } from "nostr-tools/utils";
import {
  finalizeEvent,
  generateSecretKey,
  getPublicKey,
  nip19,
  nip44,
} from "nostr-tools";

export type ViewerSecretKey = Uint8Array;

/** Generate the per-tab ephemeral nostr keypair. Memory only. */
export function generateViewerKey(): ViewerSecretKey {
  return generateSecretKey();
}

export function viewerNpub(secret: ViewerSecretKey): string {
  return nip19Npub(getPublicKey(secret));
}

export function viewerPublicKeyHex(secret: ViewerSecretKey): string {
  return getPublicKey(secret);
}

function nip19Npub(hex: string): string {
  return nip19.npubEncode(hex);
}

/** NIP-44 v2 unwrap of the Folder Key the agent wrapped to this key. */
export function unwrapFolderKey(
  secret: ViewerSecretKey,
  senderNpubOrHex: string,
  wrappedPayload: string,
): Promise<string> {
  const senderHex = senderNpubOrHex.startsWith("npub1")
    ? npubDecode(senderNpubOrHex)
    : senderNpubOrHex;
  const conversationKey = nip44.getConversationKey(secret, senderHex);
  return Promise.resolve(nip44.decrypt(wrappedPayload, conversationKey));
}

function npubDecode(npub: string): string {
  const decoded = nip19.decode(npub);
  if (decoded.type !== "npub") throw new Error("expected npub");
  return decoded.data as string;
}

async function sha256Hex(body: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(body));
  return bytesToHex(new Uint8Array(digest));
}

/**
 * Build the NIP-98 `Authorization: Nostr <base64(event)>` header the Brain
 * server validates (kind 27235, u/method tags, payload hash when a body
 * exists). Fresh nonce per call so replays never collide.
 */
export async function brainAuthHeader(
  secret: ViewerSecretKey,
  method: string,
  url: string,
  body?: string,
): Promise<string> {
  const tags: string[][] = [
    ["u", url],
    ["method", method.toUpperCase()],
    [
      "nonce",
      bytesToHex(crypto.getRandomValues(new Uint8Array(16))),
    ],
  ];
  if (body !== undefined) {
    tags.push(["payload", await sha256Hex(body)]);
  }
  const event = finalizeEvent(
    {
      kind: 27_235,
      created_at: Math.floor(Date.now() / 1000),
      tags,
      content: "",
    },
    secret,
  );
  const encoded = btoa(JSON.stringify(event));
  return `Nostr ${encoded}`;
}

/** One encrypted Folder record as returned by the encrypted-read route. */
export type ViewerRecord = {
  sequence: number;
  recordType: string;
  objectId?: string;
  revision?: number;
  ciphertext?: string;
};

/** Decrypted projection of one object: latest revision wins on replay. */
export type BrainDocObject = {
  objectId: string;
  path: string;
  markdown: string;
  revision: number;
};

type FolderObjectEnvelope = {
  version: string;
  cipher: string;
  keyVersion: number;
  nonce: string;
  ciphertext: string;
};

const FOLDER_OBJECT_VERSION = "finite-folder-object-v1";

/** Canonical AAD string the Rust envelope encrypts against. */
export function folderObjectAad(
  brainId: string,
  folderId: string,
  objectId: string,
  keyVersion: number,
): string {
  return JSON.stringify({
    version: FOLDER_OBJECT_VERSION,
    brainId,
    folderId,
    objectId,
    keyVersion,
  });
}

function base64ToBytes(value: string): Uint8Array {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function bytesToArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(copy).set(bytes);
  return copy;
}

async function importFolderKey(folderKeyBase64: string): Promise<CryptoKey> {
  return crypto.subtle.importKey(
    "raw",
    bytesToArrayBuffer(base64ToBytes(folderKeyBase64)),
    "AES-GCM",
    false,
    ["decrypt"],
  );
}

/** Open one `finite-folder-object-v1` envelope with the Folder Key. */
export async function openFolderObject(
  folderKeyBase64: string,
  brainId: string,
  folderId: string,
  objectId: string,
  envelopeJson: string,
): Promise<string> {
  const envelope: FolderObjectEnvelope = JSON.parse(envelopeJson);
  if (envelope.version !== FOLDER_OBJECT_VERSION) {
    throw new Error(`unsupported folder object envelope ${envelope.version}`);
  }
  const key = await importFolderKey(folderKeyBase64);
  const aad = new TextEncoder().encode(
    folderObjectAad(brainId, folderId, objectId, envelope.keyVersion),
  );
  const plaintext = await crypto.subtle.decrypt(
    {
      name: "AES-GCM",
      iv: base64ToBytes(envelope.nonce) as unknown as BufferSource,
      additionalData: aad as unknown as BufferSource,
    },
    key,
    bytesToArrayBuffer(base64ToBytes(envelope.ciphertext)),
  );
  return new TextDecoder().decode(plaintext);
}

type FolderObjectPlaintext = {
  version?: string;
  path?: string;
  markdown?: string;
};

/**
 * Replay folder records (sequence order) into the current object set:
 * revisions decrypt and upsert, tombstones delete. The target document is
 * the object whose decrypted path matches — the client-decrypted index
 * pattern (brain ADR-0005).
 */
export async function replayViewerRecords(
  records: ViewerRecord[],
  folderKeyBase64: string,
  brainId: string,
  folderId: string,
): Promise<Map<string, BrainDocObject>> {
  const objects = new Map<string, BrainDocObject>();
  for (const record of records) {
    const objectId = record.objectId;
    if (!objectId) continue;
    if (record.recordType === "folder_object_tombstone") {
      objects.delete(objectId);
      continue;
    }
    if (record.recordType !== "folder_object_revision" || !record.ciphertext) {
      continue;
    }
    // The record payload embeds the envelope JSON as a string under
    // "ciphertext"; the route has already extracted it for us.
    const plaintext: FolderObjectPlaintext = JSON.parse(
      await openFolderObject(
        folderKeyBase64,
        brainId,
        folderId,
        objectId,
        record.ciphertext,
      ),
    );
    if (!plaintext.path || plaintext.markdown === undefined) continue;
    objects.set(objectId, {
      objectId,
      path: plaintext.path,
      markdown: plaintext.markdown,
      revision: record.revision ?? 0,
    });
  }
  return objects;
}

/** Find the document for a brain:// path among replayed objects. */
export function findBrainDoc(
  objects: Map<string, BrainDocObject>,
  path: string,
): BrainDocObject | null {
  const normalized = path.replace(/^\/+/u, "").replace(/\/+$/u, "");
  for (const object of objects.values()) {
    if (object.path.replace(/^\/+|\/+$/gu, "") === normalized) return object;
  }
  return null;
}
