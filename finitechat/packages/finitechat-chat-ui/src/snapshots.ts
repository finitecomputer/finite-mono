export type ChatSnapshotSource = {
  generation: number;
  highestRev: number;
  hasStreamBaseline: boolean;
};

export function initialChatSnapshotSource(): ChatSnapshotSource {
  return { generation: 0, highestRev: -1, hasStreamBaseline: false };
}

/** Start a fresh stream. Its first full state is authoritative after restart. */
export function nextChatSnapshotGeneration(source: ChatSnapshotSource): ChatSnapshotSource {
  return {
    generation: source.generation + 1,
    highestRev: -1,
    hasStreamBaseline: false,
  };
}

export function shouldApplyHttpChatSnapshot(
  source: ChatSnapshotSource,
  requestGeneration: number,
  rev: number
) {
  return requestGeneration === source.generation && rev > source.highestRev;
}

export function shouldApplyStreamChatSnapshot(source: ChatSnapshotSource, rev: number) {
  return !source.hasStreamBaseline || rev > source.highestRev;
}

/**
 * Ordered streams cannot legitimately move backwards. Desktop IPC can hide a
 * planned daemon reconnect, so a lower stream revision is the restart signal
 * and its first full state must establish a fresh baseline.
 */
export function streamSnapshotNeedsNewGeneration(
  source: ChatSnapshotSource,
  rev: number,
  reconnectPending = false
) {
  return reconnectPending || (source.hasStreamBaseline && rev < source.highestRev);
}

export function recordChatSnapshot(
  source: ChatSnapshotSource,
  rev: number,
  fromStream: boolean
): ChatSnapshotSource {
  return {
    ...source,
    highestRev: rev,
    hasStreamBaseline: source.hasStreamBaseline || fromStream,
  };
}
