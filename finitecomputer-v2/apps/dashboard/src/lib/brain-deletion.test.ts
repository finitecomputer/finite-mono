import assert from "node:assert/strict";
import { test } from "node:test";

import { BrainHostedClientError } from "./brain-hosted-client";
import {
  type BrainDeletionTransport,
  brainDeletionEventTemplate,
  brainDeletionFailure,
  deleteBrainExactly,
  previewBrainDeletion,
} from "./brain-deletion";
import { BRAIN_ID_PATTERN, deletionConfirmationReady, parseBrainDeletionPreview, previewFacts } from "./brain-deletion-view";

const DIGEST = `sha256:${"ab".repeat(32)}`;

/// Shared with `finitechat-hosted-device` test
/// `hosted_device_signs_the_dashboard_brain_deletion_the_brain_server_accepts`,
/// which asserts finite-brain-core produces exactly this canonical content.
const CANONICAL_DELETE_CONTENT =
  `{"version":"finite-brain-admin-access-change-v1","brainId":"v1","changeId":"delete-brain-0123456789abcdef",` +
  `"action":"delete-brain","adminNpub":"npub1example","note":"${DIGEST}","createdAt":"2026-05-28T20:26:40Z"}`;

function previewBody(overrides: Record<string, unknown> = {}) {
  return {
    server: "https://brain.example",
    brainId: "v1",
    name: "V1",
    kind: "organization",
    folderCount: 3,
    pageCount: 12,
    memberCount: 4,
    adminCount: 2,
    guestCount: 1,
    pendingInvitationCount: 1,
    openInviteTokenCount: 2,
    blockingMounts: [],
    retiredMountRecordCount: 0,
    deletable: true,
    stateDigest: DIGEST,
    confirmationMaxAgeSeconds: 600,
    ...overrides,
  };
}

function receiptBody(duplicate: boolean) {
  return {
    duplicate,
    receipt: {
      brainId: "v1",
      deletionEventId: "e".repeat(64),
      actorNpub: "npub1example",
      deletedAt: "2026-10-25T00:00:00Z",
      stateDigest: DIGEST,
      folderCount: 3,
      pageCount: 12,
    },
  };
}

function fakeTransport(responses: Array<() => unknown>) {
  const calls: Array<{ method: string; path: string; bodyText?: string }> = [];
  const signed: unknown[] = [];
  const transport: BrainDeletionTransport = {
    signedRequest: async (method, path, bodyText) => {
      calls.push({ method, path, bodyText });
      const next = responses.shift();
      assert(next, `unexpected ${method} ${path}`);
      return next();
    },
    identify: async () => ({ npub: "npub1example" }),
    signBrainEvent: async (template) => {
      signed.push(template);
      return { id: "f".repeat(64), ...template };
    },
  };
  return { transport, calls, signed };
}

test("the delete-brain template matches finite-brain-core's canonical event", () => {
  const template = brainDeletionEventTemplate({
    brainId: "v1",
    adminNpub: "npub1example",
    stateDigest: DIGEST,
    changeId: "delete-brain-0123456789abcdef",
    createdAt: 1_780_000_000,
  });
  assert.equal(template.kind, 30_078);
  assert.equal(template.created_at, 1_780_000_000);
  assert.equal(template.content, CANONICAL_DELETE_CONTENT);
  assert.deepEqual(template.tags, [
    ["d", "finite-brain-admin-access-change:v1:delete-brain-0123456789abcdef"],
    ["brain", "v1"],
    ["action", "delete-brain"],
  ]);
  assert.throws(() =>
    brainDeletionEventTemplate({ brainId: "v1", adminNpub: "npub1x", stateDigest: "sha256:XY", changeId: "c", createdAt: 1 })
  );
});

test("deletion accepts case-sensitive Brain IDs without normalizing the confirmation", () => {
  const brainId = "Marketing-V1_Mixed";
  assert(BRAIN_ID_PATTERN.test(brainId));
  const preview = parseBrainDeletionPreview(previewBody({ brainId }));
  assert.equal(deletionConfirmationReady(preview, brainId), true);
  assert.equal(deletionConfirmationReady(preview, brainId.toLowerCase()), false);
  const event = brainDeletionEventTemplate({
    brainId, adminNpub: "npub1example", stateDigest: DIGEST, changeId: "c", createdAt: 1,
  });
  assert.equal(JSON.parse(event.content).brainId, brainId);
  for (const invalid of ["../V1", "V1/other", "V1?query", " V1"]) {
    assert.equal(BRAIN_ID_PATTERN.test(invalid), false);
  }
});

test("preview parsing keeps the exact scope and refuses inconsistent answers", async () => {
  const { transport, calls } = fakeTransport([() => previewBody()]);
  const preview = await previewBrainDeletion(transport, "v1");
  assert.deepEqual(calls, [{ method: "GET", path: "/v1/brains/v1/deletion-preview", bodyText: undefined }]);
  assert.deepEqual(previewFacts(preview), [
    ["Brain", "V1 (v1)"],
    ["Server", "https://brain.example"],
    ["Folders", "3"],
    ["Pages", "12"],
    ["Members", "4 (2 admins)"],
    ["Guests", "1"],
    ["Pending invitations", "1"],
    ["Open invite tokens", "2"],
  ]);
  assert.equal(deletionConfirmationReady(preview, "V1"), false);
  assert.equal(deletionConfirmationReady(preview, "v1 "), false);
  assert.equal(deletionConfirmationReady(preview, "v1"), true);

  const mount = {
    connectionId: "mount-1",
    direction: "incoming",
    sourceBrainId: "content-brain",
    sourceFolderId: "notes",
    destinationBrainId: "v1",
  };
  const blocked = parseBrainDeletionPreview(previewBody({ blockingMounts: [mount], deletable: false }));
  assert.equal(deletionConfirmationReady(blocked, "v1"), false);
  for (const bad of [
    previewBody({ blockingMounts: [mount], deletable: true }),
    previewBody({ stateDigest: "sha256:short" }),
    previewBody({ pageCount: -1 }),
    previewBody({ blockingMounts: [{ ...mount, direction: "sideways" }], deletable: false }),
  ]) {
    assert.throws(() => parseBrainDeletionPreview(bad));
  }
});

test("deletion signs the confirmed digest and retries only a lost transport once", async () => {
  const lost = fakeTransport([
    () => { throw new TypeError("fetch failed"); },
    () => receiptBody(true),
  ]);
  const result = await deleteBrainExactly(lost.transport, { brainId: "v1", stateDigest: DIGEST }, 1_780_000_000);
  assert.equal(result.duplicate, true);
  assert.equal(result.receipt.stateDigest, DIGEST);
  assert.equal(lost.signed.length, 1);
  assert.equal(lost.calls.length, 2);
  assert.equal(lost.calls[0].method, "DELETE");
  assert.equal(lost.calls[0].path, "/v1/brains/v1");
  // The retry is the identical signed deletion, not a new one.
  assert.equal(lost.calls[1].bodyText, lost.calls[0].bodyText);
  const event = JSON.parse(lost.calls[0].bodyText ?? "{}").deletionEvent;
  assert.equal(JSON.parse(event.content).note, DIGEST);
  assert.equal(JSON.parse(event.content).action, "delete-brain");

  // A Brain server answer is final.
  const refused = fakeTransport([
    () => { throw new BrainHostedClientError("Brain changed after its deletion preview", 409); },
  ]);
  await assert.rejects(
    deleteBrainExactly(refused.transport, { brainId: "v1", stateDigest: DIGEST }),
    (error) => error instanceof BrainHostedClientError && error.status === 409
  );
  assert.equal(refused.calls.length, 1);
});

test("refusals keep the Brain server's status and message", () => {
  for (const status of [403, 404, 409, 410, 400]) {
    assert.deepEqual(brainDeletionFailure(new BrainHostedClientError(`server says ${status}`, status)), {
      status,
      message: `server says ${status}`,
    });
  }
  assert.equal(brainDeletionFailure(new BrainHostedClientError("boom", 500)).status, 502);
  assert.equal(brainDeletionFailure(new TypeError("fetch failed")).status, 502);
});
