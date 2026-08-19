import assert from "node:assert/strict";
import test from "node:test";

import { getPublicKey, nip44 } from "nostr-tools";
import {
  brainAuthHeader,
  findBrainDoc,
  folderObjectAad,
  generateViewerKey,
  openFolderObject,
  replayViewerRecords,
  unwrapFolderKey,
  viewerNpub,
} from "./brain-viewer-crypto";

const fromHex = (hex: string) =>
  new Uint8Array(hex.match(/.{2}/gu)!.map((byte) => Number.parseInt(byte, 16)));

function toBase64(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("base64");
}

/** Folder key used by the Rust core test vectors: 32 bytes of 0x09. */
const FOLDER_KEY_B64 = toBase64(new Uint8Array(32).fill(9));

/** Build a finite-folder-object-v1 envelope the way the Rust core does. */
async function rustStyleEnvelope(
  brainId: string,
  folderId: string,
  objectId: string,
  keyVersion: number,
  plaintext: string,
): Promise<string> {
  const key = await crypto.subtle.importKey(
    "raw",
    fromHex("0909090909090909090909090909090909090909090909090909090909090909"),
    "AES-GCM",
    false,
    ["encrypt"],
  );
  const nonce = new Uint8Array(12).fill(7);
  const aad = new TextEncoder().encode(
    folderObjectAad(brainId, folderId, objectId, keyVersion),
  );
  const ciphertext = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: nonce as unknown as BufferSource, additionalData: aad as unknown as BufferSource },
    key,
    new TextEncoder().encode(plaintext),
  );
  return JSON.stringify({
    version: "finite-folder-object-v1",
    cipher: "AES-256-GCM",
    keyVersion,
    nonce: toBase64(nonce),
    ciphertext: toBase64(new Uint8Array(ciphertext)),
  });
}

test("viewer keys are fresh nostr keys with npub identities", () => {
  const first = generateViewerKey();
  const second = generateViewerKey();
  assert.notEqual(Buffer.from(first).toString("hex"), Buffer.from(second).toString("hex"));
  const npub = viewerNpub(first);
  assert.match(npub, /^npub1/u);
});

test("folder object envelopes decrypt against the canonical Rust AAD", async () => {
  const envelope = await rustStyleEnvelope("acme", "general", "obj_000000000501", 1, "hello viewer");
  const plaintext = await openFolderObject(
    FOLDER_KEY_B64,
    "acme",
    "general",
    "obj_000000000501",
    envelope,
  );
  assert.equal(plaintext, "hello viewer");
});

test("envelope decryption fails closed on a mismatched AAD", async () => {
  const envelope = await rustStyleEnvelope("acme", "general", "obj_000000000501", 1, "secret");
  await assert.rejects(
    openFolderObject(FOLDER_KEY_B64, "other-brain", "general", "obj_000000000501", envelope),
  );
});

test("wrapped folder keys unwrap with the ephemeral secret and sender npub", async () => {
  const agentSecret = generateViewerKey();
  const viewerSecret = generateViewerKey();
  const agentNpub = viewerNpub(agentSecret);
  const wrapped = nip44.encrypt(
    FOLDER_KEY_B64,
    nip44.getConversationKey(agentSecret, getPublicKey(viewerSecret)),
  );
  const unwrapped = await unwrapFolderKey(viewerSecret, agentNpub, wrapped);
  assert.equal(unwrapped, FOLDER_KEY_B64);
});

test("replay builds the client-decrypted path index and finds the target doc", async () => {
  const first = await rustStyleEnvelope(
    "acme",
    "general",
    "obj_000000000501",
    1,
    JSON.stringify({ version: "finite-folder-object-page-v1", path: "roadmap.md", markdown: "# v1" }),
  );
  const second = await rustStyleEnvelope(
    "acme",
    "general",
    "obj_000000000501",
    1,
    JSON.stringify({ version: "finite-folder-object-page-v1", path: "roadmap.md", markdown: "# v2 edited" }),
  );
  const other = await rustStyleEnvelope(
    "acme",
    "general",
    "obj_000000000502",
    1,
    JSON.stringify({ version: "finite-folder-object-page-v1", path: "notes/other.md", markdown: "other" }),
  );
  const objects = await replayViewerRecords(
    [
      { sequence: 1, recordType: "folder_object_revision", objectId: "obj_000000000501", revision: 1, ciphertext: first },
      { sequence: 2, recordType: "folder_object_revision", objectId: "obj_000000000502", revision: 1, ciphertext: other },
      { sequence: 3, recordType: "folder_object_revision", objectId: "obj_000000000501", revision: 2, ciphertext: second },
      { sequence: 4, recordType: "folder_object_tombstone", objectId: "obj_000000000502", revision: 2 },
    ],
    FOLDER_KEY_B64,
    "acme",
    "general",
  );
  assert.equal(objects.size, 1, "tombstone must remove the deleted object");
  const doc = findBrainDoc(objects, "roadmap.md");
  assert.ok(doc);
  assert.equal(doc.markdown, "# v2 edited");
  assert.equal(findBrainDoc(objects, "notes/other.md"), null);
});

test("auth headers carry a NIP-98 kind 27235 event with u and method tags", async () => {
  const secret = generateViewerKey();
  const header = await brainAuthHeader(secret, "GET", "http://brain.test/v1/records");
  assert.ok(header.startsWith("Nostr "));
  const event = JSON.parse(Buffer.from(header.slice(6), "base64").toString("utf8")) as {
    kind: number;
    tags: string[][];
    pubkey: string;
    sig: string;
  };
  assert.equal(event.kind, 27_235);
  assert.deepEqual(event.tags[0], ["u", "http://brain.test/v1/records"]);
  assert.deepEqual(event.tags[1], ["method", "GET"]);
  assert.equal(event.pubkey, getPublicKey(secret));
  assert.equal(typeof event.sig, "string");
});

test("auth headers hash the payload when a body is present", async () => {
  const secret = generateViewerKey();
  const body = JSON.stringify({ brainId: "acme" });
  const withBody = await brainAuthHeader(secret, "POST", "http://brain.test/v1/x", body);
  const withoutBody = await brainAuthHeader(secret, "POST", "http://brain.test/v1/x");
  const event = JSON.parse(Buffer.from(withBody.slice(6), "base64").toString("utf8")) as {
    tags: string[][];
  };
  const payloadTag = event.tags.find((tag) => tag[0] === "payload");
  assert.ok(payloadTag, "payload tag must be present");
  const digest = Buffer.from(
    await crypto.subtle.digest("SHA-256", new TextEncoder().encode(body)),
  ).toString("hex");
  assert.equal(payloadTag[1], digest);
  const bare = JSON.parse(Buffer.from(withoutBody.slice(6), "base64").toString("utf8")) as {
    tags: string[][];
  };
  assert.equal(bare.tags.find((tag) => tag[0] === "payload"), undefined);
});
