// Finite Chat invite surface for a hosted agent runtime.
//
// The runtime's health server publishes its invite status at
// `{public_base_url}/invite` (the first published app URL in Core's runtime
// facts). GET returns `{ready, room_id, invite_id, url, error?}` where `url`
// is a `finite://join?...` invite code. This module owns fetching and
// validating that payload server-side — the dashboard client never talks to
// the runtime origin — plus the render-state decision for the invite card.
//
// Phase 3 swaps the public fetch for a pairing-credential fetch; keep every
// runtime-origin detail behind this module so only this file changes.

export const AGENT_INVITE_FETCH_TIMEOUT_MS = 5_000;
export const AGENT_INVITE_WAIT_TIMEOUT_MS = 90_000;
export const AGENT_INVITE_POLL_INTERVAL_MS = 2_000;
export const AGENT_INVITE_MAX_POLL_INTERVAL_MS = 10_000;

export type AgentInviteStatus =
  | { state: "ready"; inviteUrl: string; roomId: string | null }
  | { state: "paired"; roomId: string | null; agentNpub: string | null }
  | { state: "pending" }
  | { state: "error"; message: string };

// Only `finite://join?...` URIs may reach an href or QR code.
export function isFiniteJoinUrl(value: unknown): value is string {
  if (typeof value !== "string" || !value.trim()) {
    return false;
  }
  try {
    const url = new URL(value);
    return url.protocol === "finite:" && url.host === "join";
  } catch {
    return false;
  }
}

export function parseAgentInviteResponse(payload: unknown): AgentInviteStatus {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    return { state: "pending" };
  }
  const record = payload as Record<string, unknown>;

  // A confirmed pairing beats any URL the payload might still carry — never
  // re-show a QR for a paired agent.
  if (record.paired === true) {
    return {
      state: "paired",
      roomId: optionalTrimmedString(record.room_id),
      agentNpub: optionalTrimmedString(record.agent_npub),
    };
  }

  // A single-use invite may be consumed before the MLS Welcome/admission is
  // observable by the runtime. That is not paired yet, and there is no safe QR
  // to re-render; keep the bounded waiting state instead of showing success or
  // a broken invite error.
  if (record.invite_state === "consumed_pending_admission") {
    return { state: "pending" };
  }

  // The runtime lost its invite session entirely; waiting will never help,
  // the user needs a fresh invite.
  if (record.invite_state === "not_found") {
    return {
      state: "error",
      message:
        "The runtime has no invite session for this agent. Restart the agent to issue a new invite.",
    };
  }

  if (record.ready !== true) {
    const message = typeof record.error === "string" ? record.error.trim() : "";
    return message ? { state: "error", message } : { state: "pending" };
  }

  if (!isFiniteJoinUrl(record.url)) {
    // Ready without a valid join URL must never render a broken QR code.
    return {
      state: "error",
      message: "The runtime reported an invite without a valid finite:// join link.",
    };
  }

  return {
    state: "ready",
    inviteUrl: record.url,
    roomId: optionalTrimmedString(record.room_id),
  };
}

function optionalTrimmedString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

// Fetches the runtime's invite status server-side. Network failures,
// timeouts, and malformed payloads all read as "pending": the invite simply
// is not ready to show yet, and the bounded refresh keeps checking.
export async function fetchAgentInvite(
  inviteStatusUrl: string | null | undefined,
  options: { timeoutMs?: number } = {}
): Promise<AgentInviteStatus> {
  if (!safeHttpUrl(inviteStatusUrl)) {
    return { state: "pending" };
  }

  try {
    const response = await fetch(inviteStatusUrl, {
      cache: "no-store",
      headers: { accept: "application/json" },
      signal: AbortSignal.timeout(options.timeoutMs ?? AGENT_INVITE_FETCH_TIMEOUT_MS),
    });
    if (!response.ok) {
      return { state: "pending" };
    }
    return parseAgentInviteResponse(JSON.parse(await response.text()));
  } catch {
    return { state: "pending" };
  }
}

export type AgentInviteDisplay =
  // Render the QR + copy + open-in-Finite-Chat surface.
  | { kind: "ready"; inviteUrl: string; roomId: string | null }
  // The invite was consumed: show the success card, no QR and no polling.
  | { kind: "paired"; roomId: string | null; agentNpub: string | null }
  // Invite pending and no bounded wait window stamped yet: redirect once to
  // stamp the window start (same pattern as the billing sync wait).
  | { kind: "stamp-wait-start" }
  // Invite pending inside the wait window: show the waiting state and poll.
  | { kind: "waiting"; deadlineAtMs: number }
  // Still pending after the window: stop polling, offer a manual re-check.
  | { kind: "wait-timeout" }
  // The runtime reported an invite error: show it with a retry affordance.
  | { kind: "error"; message: string };

export type AgentInviteDisplayInput = {
  invite: AgentInviteStatus;
  // Epoch ms stamped into the URL when the pending invite first rendered.
  waitStartedAtMs: number | null;
  nowMs: number;
  timeoutMs?: number;
};

export function resolveAgentInviteDisplay(input: AgentInviteDisplayInput): AgentInviteDisplay {
  if (input.invite.state === "ready") {
    return {
      kind: "ready",
      inviteUrl: input.invite.inviteUrl,
      roomId: input.invite.roomId,
    };
  }
  if (input.invite.state === "paired") {
    return {
      kind: "paired",
      roomId: input.invite.roomId,
      agentNpub: input.invite.agentNpub,
    };
  }
  if (input.invite.state === "error") {
    return { kind: "error", message: input.invite.message };
  }

  if (input.waitStartedAtMs === null) {
    return { kind: "stamp-wait-start" };
  }
  const deadlineAtMs = input.waitStartedAtMs + (input.timeoutMs ?? AGENT_INVITE_WAIT_TIMEOUT_MS);
  if (input.nowMs < deadlineAtMs) {
    return { kind: "waiting", deadlineAtMs };
  }
  return { kind: "wait-timeout" };
}

// Clock-reading wrapper for server components, where the purity lint bans
// direct Date.now() calls during render.
export function resolveAgentInviteDisplayNow(
  input: Omit<AgentInviteDisplayInput, "nowMs">
): AgentInviteDisplay {
  return resolveAgentInviteDisplay({ ...input, nowMs: Date.now() });
}

// Redirect target that stamps the start of the bounded invite wait window.
export function agentInviteWaitStampRedirectPath(
  machineId: string,
  nowMs: number = Date.now()
) {
  return `/dashboard/machines/${encodeURIComponent(machineId)}?inviteWaitStartedAt=${nowMs}`;
}

export function parseAgentInviteWaitStartedAt(
  value: string | null | undefined
): number | null {
  if (!value?.trim()) {
    return null;
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) {
    return null;
  }
  return parsed;
}

export function truncateInviteUrl(value: string, maxLength = 44) {
  if (value.length <= maxLength) {
    return value;
  }
  return `${value.slice(0, maxLength)}…`;
}

// Middle-truncates an npub for display: the bech32 prefix and checksum tail
// are what humans eyeball-compare.
export function truncateNpub(value: string, headLength = 12, tailLength = 6) {
  if (value.length <= headLength + tailLength + 1) {
    return value;
  }
  return `${value.slice(0, headLength)}…${value.slice(-tailLength)}`;
}

function safeHttpUrl(value: string | null | undefined): value is string {
  if (!value?.trim()) {
    return false;
  }
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:";
  } catch {
    return false;
  }
}
