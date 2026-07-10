export type HostedChatSnapshotSource = {
  generation: number;
  highestRev: number;
  hasStreamBaseline: boolean;
};

export function initialHostedChatSnapshotSource(): HostedChatSnapshotSource {
  return { generation: 0, highestRev: -1, hasStreamBaseline: false };
}

/** Start a fresh SSE source. Its first full state is authoritative, even after a restart. */
export function nextHostedChatSnapshotGeneration(
  source: HostedChatSnapshotSource
): HostedChatSnapshotSource {
  return {
    generation: source.generation + 1,
    highestRev: -1,
    hasStreamBaseline: false,
  };
}

export function shouldApplyHttpHostedChatSnapshot(
  source: HostedChatSnapshotSource,
  requestGeneration: number,
  rev: number
) {
  return requestGeneration === source.generation && rev > source.highestRev;
}

export function shouldApplyStreamHostedChatSnapshot(
  source: HostedChatSnapshotSource,
  rev: number
) {
  return !source.hasStreamBaseline || rev > source.highestRev;
}

export function recordHostedChatSnapshot(
  source: HostedChatSnapshotSource,
  rev: number,
  fromStream: boolean
): HostedChatSnapshotSource {
  return {
    ...source,
    highestRev: rev,
    hasStreamBaseline: source.hasStreamBaseline || fromStream,
  };
}
