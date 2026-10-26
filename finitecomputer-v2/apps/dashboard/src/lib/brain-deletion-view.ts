/// Organization Brain deletion shapes shared by the dashboard route and the
/// deletion panel: parsing of the Brain server's preview and receipt, and the
/// confirmation rule. No server-only imports.

export const BRAIN_ID_PATTERN = /^[a-z0-9][a-z0-9_-]{0,127}$/u;
export const STATE_DIGEST_PATTERN = /^sha256:[0-9a-f]{64}$/u;

export type BlockingMount = {
  connectionId: string;
  direction: "outgoing" | "incoming";
  sourceBrainId: string;
  sourceFolderId: string;
  destinationBrainId: string;
};

export type BrainDeletionPreview = {
  server: string;
  brainId: string;
  name: string;
  folderCount: number;
  pageCount: number;
  memberCount: number;
  adminCount: number;
  guestCount: number;
  pendingInvitationCount: number;
  openInviteTokenCount: number;
  blockingMounts: BlockingMount[];
  retiredMountRecordCount: number;
  deletable: boolean;
  stateDigest: string;
  confirmationMaxAgeSeconds: number;
};

export type BrainDeletionReceipt = {
  brainId: string;
  deletionEventId: string;
  actorNpub: string;
  deletedAt: string;
  stateDigest: string;
  folderCount: number;
  pageCount: number;
};

export type BrainDeletionResult = { receipt: BrainDeletionReceipt; duplicate: boolean };

export class InvalidBrainResponse extends Error {}

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new InvalidBrainResponse("Brain returned an unexpected deletion response.");
  }
  return value as Record<string, unknown>;
}

function text(value: unknown): string {
  if (typeof value !== "string" || !value || value.length > 2048) {
    throw new InvalidBrainResponse("Brain returned an unexpected deletion response.");
  }
  return value;
}

function count(value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new InvalidBrainResponse("Brain returned an unexpected deletion response.");
  }
  return value;
}

function digest(value: unknown): string {
  const candidate = text(value);
  if (!STATE_DIGEST_PATTERN.test(candidate)) {
    throw new InvalidBrainResponse("Brain returned an unexpected deletion response.");
  }
  return candidate;
}

export function parseBrainDeletionPreview(value: unknown): BrainDeletionPreview {
  const data = record(value);
  if (!Array.isArray(data.blockingMounts) || data.blockingMounts.length > 2_000) {
    throw new InvalidBrainResponse("Brain returned an unexpected deletion response.");
  }
  const blockingMounts = data.blockingMounts.map((entry): BlockingMount => {
    const mount = record(entry);
    if (mount.direction !== "outgoing" && mount.direction !== "incoming") {
      throw new InvalidBrainResponse("Brain returned an unexpected deletion response.");
    }
    return {
      connectionId: text(mount.connectionId),
      direction: mount.direction,
      sourceBrainId: text(mount.sourceBrainId),
      sourceFolderId: text(mount.sourceFolderId),
      destinationBrainId: text(mount.destinationBrainId),
    };
  });
  if (typeof data.deletable !== "boolean" || data.deletable !== (blockingMounts.length === 0)) {
    throw new InvalidBrainResponse("Brain returned an unexpected deletion response.");
  }
  return {
    server: text(data.server),
    brainId: text(data.brainId),
    name: text(data.name),
    folderCount: count(data.folderCount),
    pageCount: count(data.pageCount),
    memberCount: count(data.memberCount),
    adminCount: count(data.adminCount),
    guestCount: count(data.guestCount),
    pendingInvitationCount: count(data.pendingInvitationCount),
    openInviteTokenCount: count(data.openInviteTokenCount),
    blockingMounts,
    retiredMountRecordCount: count(data.retiredMountRecordCount),
    deletable: data.deletable,
    stateDigest: digest(data.stateDigest),
    confirmationMaxAgeSeconds: count(data.confirmationMaxAgeSeconds),
  };
}

export function parseBrainDeletionResult(value: unknown): BrainDeletionResult {
  const data = record(value);
  const receipt = record(data.receipt);
  if (typeof data.duplicate !== "boolean") {
    throw new InvalidBrainResponse("Brain returned an unexpected deletion response.");
  }
  return {
    duplicate: data.duplicate,
    receipt: {
      brainId: text(receipt.brainId),
      deletionEventId: text(receipt.deletionEventId),
      actorNpub: text(receipt.actorNpub),
      deletedAt: text(receipt.deletedAt),
      stateDigest: digest(receipt.stateDigest),
      folderCount: count(receipt.folderCount),
      pageCount: count(receipt.pageCount),
    },
  };
}

/// Deletion is allowed only for a deletable preview whose Brain ID the
/// account typed exactly.
export function deletionConfirmationReady(preview: BrainDeletionPreview, typed: string) {
  return preview.deletable && typed === preview.brainId;
}

/// The exact scope the account confirms, in display order.
export function previewFacts(preview: BrainDeletionPreview): Array<[string, string]> {
  return [
    ["Brain", `${preview.name} (${preview.brainId})`],
    ["Server", preview.server],
    ["Folders", String(preview.folderCount)],
    ["Pages", String(preview.pageCount)],
    ["Members", `${preview.memberCount} (${preview.adminCount} ${preview.adminCount === 1 ? "admin" : "admins"})`],
    ["Guests", String(preview.guestCount)],
    ["Pending invitations", String(preview.pendingInvitationCount)],
    ["Open invite tokens", String(preview.openInviteTokenCount)],
  ];
}
