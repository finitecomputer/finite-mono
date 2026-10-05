/// The Brain server's invitee-scoped invitation list (`GET /v1/my-invitations`,
/// relayed unchanged by `/api/brain/invitations`). It returns only pending,
/// npub-addressed Brain invitations; there is no `status` field. `expired` is
/// computed by the server at read time.

export type BrainInvitationCardData = {
  id: string;
  brainId: string;
  brainDisplayName: string | null;
  inviteCode: string;
  expired: boolean;
};

/// Same shape the accept route requires before it signs anything.
const INVITE_CODE = /^[a-z0-9][a-z0-9_-]{0,127}$/u;
const MAX_CARDS = 50;
const MAX_TEXT = 200;

function boundedText(value: unknown): string | null {
  return typeof value === "string" && value.trim() && value.length <= MAX_TEXT
    ? value.trim()
    : null;
}

/// Cards for the server response. Rows without an id, Brain id or a valid
/// invite code are dropped; anything that is not the expected shape yields
/// no cards.
export function invitationCardsFromResponse(body: unknown): BrainInvitationCardData[] {
  const listed =
    body && typeof body === "object" ? (body as { invitations?: unknown }).invitations : null;
  if (!Array.isArray(listed)) return [];
  const cards: BrainInvitationCardData[] = [];
  for (const row of listed) {
    if (cards.length >= MAX_CARDS) break;
    if (!row || typeof row !== "object") continue;
    const record = row as Record<string, unknown>;
    const id = boundedText(record.id);
    const brainId = boundedText(record.brainId);
    const inviteCode = typeof record.inviteCode === "string" ? record.inviteCode : "";
    if (!id || !brainId || !INVITE_CODE.test(inviteCode)) continue;
    cards.push({
      id,
      brainId,
      brainDisplayName: boundedText(record.brainDisplayName),
      inviteCode,
      expired: record.expired === true,
    });
  }
  return cards;
}

/// The Join request the card sends: the invite code plus the explicit intent
/// that goes with the card's contact-sharing text.
export function joinRequestBody(card: BrainInvitationCardData) {
  return { inviteCode: card.inviteCode, shareAccountContact: true as const };
}
