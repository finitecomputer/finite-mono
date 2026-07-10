import assert from "node:assert/strict";
import test from "node:test";

import {
  initialHostedChatSnapshotSource,
  nextHostedChatSnapshotGeneration,
  recordHostedChatSnapshot,
  shouldApplyHttpHostedChatSnapshot,
  shouldApplyStreamHostedChatSnapshot,
} from "@/lib/hosted-web-chat-snapshots";

test("an older HTTP snapshot cannot overwrite a newer SSE snapshot", () => {
  let source = initialHostedChatSnapshotSource();
  const requestGeneration = source.generation;
  source = nextHostedChatSnapshotGeneration(source);

  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 12), true);
  source = recordHostedChatSnapshot(source, 12, true);
  assert.equal(shouldApplyHttpHostedChatSnapshot(source, requestGeneration, 11), false);
  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 11), false);
});

test("the first full SSE state after reconnect establishes a lower revision baseline", () => {
  let source = initialHostedChatSnapshotSource();
  source = nextHostedChatSnapshotGeneration(source);
  source = recordHostedChatSnapshot(source, 42, true);
  source = nextHostedChatSnapshotGeneration(source);

  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 3), true);
  source = recordHostedChatSnapshot(source, 3, true);
  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 3), false);
  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 4), true);
});
