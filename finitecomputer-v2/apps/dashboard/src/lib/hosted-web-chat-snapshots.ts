import {
  shouldApplyHttpChatSnapshot,
  type ChatSnapshotSource,
} from "@finite/chat-ui";

export {
  initialChatSnapshotSource as initialHostedChatSnapshotSource,
  nextChatSnapshotGeneration as nextHostedChatSnapshotGeneration,
  recordChatSnapshot as recordHostedChatSnapshot,
  shouldApplyHttpChatSnapshot as shouldApplyHttpHostedChatSnapshot,
  type ChatSnapshotSource as HostedChatSnapshotSource,
} from "@finite/chat-ui";

export type HostedChatStreamConnection = {
  generation: number;
  highestRevAtConnect: number;
  snapshotSequenceAtConnect: number;
};

export function beginHostedChatStreamConnection(
  source: ChatSnapshotSource,
  snapshotSequence: number
): {
  source: ChatSnapshotSource;
  connection: HostedChatStreamConnection;
} {
  return {
    source: { ...source, hasStreamBaseline: false },
    connection: {
      generation: source.generation,
      highestRevAtConnect: source.highestRev,
      snapshotSequenceAtConnect: snapshotSequence,
    },
  };
}

export function hostedChatStreamSnapshotProvesRestart(
  source: ChatSnapshotSource,
  connection: HostedChatStreamConnection,
  snapshotSequence: number,
  rev: number
) {
  return !source.hasStreamBaseline
    && source.generation === connection.generation
    && source.highestRev === connection.highestRevAtConnect
    && snapshotSequence === connection.snapshotSequenceAtConnect
    && rev < connection.highestRevAtConnect;
}

/**
 * The first event on an SSE connection is a baseline, but it is not allowed to
 * roll back a newer HTTP response that completed while that event was delayed.
 * A lower first event proves a restart only when no HTTP or mutation snapshot
 * has landed since this connection captured its starting revision.
 */
export function shouldApplyStreamHostedChatSnapshot(
  source: ChatSnapshotSource,
  rev: number,
  snapshotAdvancedWhileBaselinePending = false
) {
  return source.hasStreamBaseline || snapshotAdvancedWhileBaselinePending
    ? rev > source.highestRev
    : rev >= source.highestRev;
}

export function shouldApplyMutationHostedChatSnapshot(
  source: ChatSnapshotSource,
  requestGeneration: number,
  requestHighestRev: number,
  requestSequence: number,
  latestAppliedMutationSequence: number,
  allowEqualRevision: boolean,
  rev: number
) {
  return shouldApplyHttpChatSnapshot(source, requestGeneration, rev)
    || (
      source.generation === requestGeneration
      && allowEqualRevision
      && source.highestRev === requestHighestRev
      && rev === source.highestRev
      && requestSequence > latestAppliedMutationSequence
    );
}
