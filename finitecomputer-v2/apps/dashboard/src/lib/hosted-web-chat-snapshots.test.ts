import assert from "node:assert/strict";
import test from "node:test";

import {
  beginHostedChatStreamConnection,
  hostedChatStreamSnapshotProvesRestart,
  initialHostedChatSnapshotSource,
  nextHostedChatSnapshotGeneration,
  recordHostedChatSnapshot,
  shouldApplyHttpHostedChatSnapshot,
  shouldApplyMutationHostedChatSnapshot,
  shouldApplyStreamHostedChatSnapshot,
} from "@/lib/hosted-web-chat-snapshots";
import { streamSnapshotNeedsNewGeneration } from "@finite/chat-ui";

test("an older HTTP snapshot cannot overwrite a newer SSE snapshot", () => {
  let source = initialHostedChatSnapshotSource();
  const requestGeneration = source.generation;
  source = nextHostedChatSnapshotGeneration(source);

  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 12), true);
  source = recordHostedChatSnapshot(source, 12, true);
  assert.equal(shouldApplyHttpHostedChatSnapshot(source, requestGeneration, 11), false);
  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 11), false);
});

test("an unchanged lower first SSE proves restart and establishes a new baseline", () => {
  let source = initialHostedChatSnapshotSource();
  source = recordHostedChatSnapshot(source, 42, true);
  const stream = beginHostedChatStreamConnection(source, 1);
  source = stream.source;

  assert.equal(hostedChatStreamSnapshotProvesRestart(source, stream.connection, 1, 3), true);
  source = nextHostedChatSnapshotGeneration(source);
  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 3), true);
  source = recordHostedChatSnapshot(source, 3, true);
  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 3), false);
  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 4), true);
});

test("a delayed initial SSE baseline cannot roll back a completed mutation response", () => {
  let source = initialHostedChatSnapshotSource();
  source = recordHostedChatSnapshot(source, 7, false);
  const stream = beginHostedChatStreamConnection(source, 1);
  source = stream.source;
  const requestGeneration = source.generation;
  const requestHighestRev = source.highestRev;

  assert.equal(
    shouldApplyMutationHostedChatSnapshot(
      source,
      requestGeneration,
      requestHighestRev,
      1,
      0,
      true,
      8
    ),
    true
  );
  source = recordHostedChatSnapshot(source, 8, false);

  assert.equal(hostedChatStreamSnapshotProvesRestart(source, stream.connection, 2, 7), false);
  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 7, true), false);
  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 8), true);
});

test("a delayed equal-revision initial SSE cannot undo a selection mutation", () => {
  let source = initialHostedChatSnapshotSource();
  source = recordHostedChatSnapshot(source, 7, false);
  const stream = beginHostedChatStreamConnection(source, 1);
  source = stream.source;

  assert.equal(
    shouldApplyMutationHostedChatSnapshot(source, source.generation, 7, 1, 0, true, 7),
    true
  );
  source = recordHostedChatSnapshot(source, 7, false);

  assert.equal(hostedChatStreamSnapshotProvesRestart(source, stream.connection, 2, 6), false);
  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 7, true), false);
  assert.equal(shouldApplyStreamHostedChatSnapshot(source, 8, true), true);
});

test("a silent desktop daemon restart is detected by its lower ordered stream revision", () => {
  let source = initialHostedChatSnapshotSource();
  source = nextHostedChatSnapshotGeneration(source);
  source = recordHostedChatSnapshot(source, 42, true);

  assert.equal(streamSnapshotNeedsNewGeneration(source, 3), true);
  assert.equal(streamSnapshotNeedsNewGeneration(source, 43), false);
});

test("a selection mutation can replace an unchanged equal-revision snapshot", () => {
  let source = initialHostedChatSnapshotSource();
  source = recordHostedChatSnapshot(source, 7, true);

  assert.equal(
    shouldApplyMutationHostedChatSnapshot(
      source,
      source.generation,
      source.highestRev,
      2,
      1,
      true,
      7
    ),
    true
  );
});

test("an equal-revision mutation cannot overwrite a snapshot that advanced in flight", () => {
  let source = initialHostedChatSnapshotSource();
  source = recordHostedChatSnapshot(source, 7, true);
  const requestGeneration = source.generation;
  const requestHighestRev = source.highestRev;
  source = recordHostedChatSnapshot(source, 8, true);

  assert.equal(
    shouldApplyMutationHostedChatSnapshot(
      source,
      requestGeneration,
      requestHighestRev,
      2,
      1,
      true,
      7
    ),
    false
  );
  assert.equal(
    shouldApplyMutationHostedChatSnapshot(
      source,
      requestGeneration,
      requestHighestRev,
      2,
      1,
      true,
      8
    ),
    false
  );
});

test("an older equal-revision mutation response cannot overwrite a newer response", () => {
  let source = initialHostedChatSnapshotSource();
  source = recordHostedChatSnapshot(source, 7, true);

  assert.equal(
    shouldApplyMutationHostedChatSnapshot(source, source.generation, 7, 2, 0, true, 7),
    true
  );
  assert.equal(
    shouldApplyMutationHostedChatSnapshot(source, source.generation, 7, 1, 2, true, 7),
    false
  );
});

test("an equal-revision quiet response cannot suppress foreground navigation", () => {
  let source = initialHostedChatSnapshotSource();
  source = recordHostedChatSnapshot(source, 7, true);

  assert.equal(
    shouldApplyMutationHostedChatSnapshot(source, source.generation, 7, 2, 0, false, 7),
    false
  );
  assert.equal(
    shouldApplyMutationHostedChatSnapshot(source, source.generation, 7, 1, 0, true, 7),
    true
  );
});
