import {
  HostedHermesStatusError, readHostedHermesJson, sendHostedHermesChange,
  type HostedHermesChange, type HostedHermesChangeResult,
} from "./hosted-hermes-status";

/** Admin roster changes the selected Agent signs through its finite-brain
 * plugin: invite by npub or public name, revoke a pending invitation, and
 * remove a Member or Admin. Each change is sent once and never retried. */

export type RosterAction = "invite" | "revokeInvitation" | "remove";
/** Absent (null) from an older Agent plugin: show no buttons. */
export type RosterChanges = { actingNpub: string | null; actions: RosterAction[] };
export type PendingInvitation = {
  id: string; targetNpub: string | null; targetName: string | null; expiresAt: string; expired: boolean;
};
export type RemovalState = "running" | "removed" | "demotedNotRemoved" | "failed" | "unknown";
export type ChangeErrorCode =
  | "busy" | "notAdmin" | "invalidInput" | "notPending" | "notRemovable" | "ownKey"
  | "failed" | "unknown" | "unavailable" | "access";

const NPUB = /^npub1[023456789acdefghjklmnpqrstuvwxyz]{58}$/;
// Matches the plugin: a lowercase public NIP-05 name that never looks like a flag.
const NIP05 = /^[a-z0-9_][a-z0-9._-]{0,63}@[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$/;
const BRAIN_ID = /^[A-Za-z0-9_][A-Za-z0-9_-]{0,127}$/;
const INVITATION_ID = /^invitation-[0-9a-f]{32}$/;
const JOB_ID = /^[0-9a-f]{32}$/;
const ACTIONS: RosterAction[] = ["invite", "revokeInvitation", "remove"];
const REMOVAL_STATES = new Set<RemovalState>(["running", "removed", "demotedNotRemoved", "failed", "unknown"]);
const ERROR_CODES = new Set<ChangeErrorCode>([
  "busy", "notAdmin", "invalidInput", "notPending", "notRemovable", "ownKey", "failed", "unknown", "unavailable",
]);

export class RosterChangeError extends Error {
  constructor(readonly code: ChangeErrorCode, message: string = code) {
    super(message);
    this.name = "RosterChangeError";
  }
}

function invalid(): never {
  throw new HostedHermesStatusError("This agent needs a compatible page update. Try again after it updates.", "unsupported");
}
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
  return value as Record<string, unknown>;
}

export const shortNpub = (npub: string) => `${npub.slice(0, 12)}…${npub.slice(-6)}`;

/** The identities response's optional `changes` field. */
export function parseRosterChanges(value: unknown): RosterChanges | null {
  const changes = record(value).changes;
  if (changes == null) return null;
  const data = record(changes);
  const actingNpub = data.actingNpub ?? null;
  if (actingNpub !== null && (typeof actingNpub !== "string" || !NPUB.test(actingNpub))) return invalid();
  if (!Array.isArray(data.actions) || data.actions.length > 16) return invalid();
  // Unknown future actions are ignored; known ones need the acting key.
  const actions = ACTIONS.filter(action => (data.actions as unknown[]).includes(action));
  return { actingNpub, actions: actingNpub ? actions : [] };
}

/** Trim and lowercase; null unless it is an npub or a public NIP-05 name. */
export function inviteTarget(input: string): string | null {
  const value = input.trim().toLowerCase();
  if (NPUB.test(value) || (value.length <= 254 && NIP05.test(value))) return value;
  return null;
}

const rfc3339 = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$/;
function invitation(value: unknown): PendingInvitation {
  const item = record(value);
  const { id, targetNpub, targetName, expiresAt, expired } = item;
  if (typeof id !== "string" || !INVITATION_ID.test(id)) return invalid();
  if (targetNpub !== null && (typeof targetNpub !== "string" || !NPUB.test(targetNpub))) return invalid();
  if (targetName !== null && (typeof targetName !== "string" || targetName.length > 254 || !NIP05.test(targetName))) return invalid();
  if (typeof expiresAt !== "string" || !rfc3339.test(expiresAt) || Number.isNaN(Date.parse(expiresAt))) return invalid();
  if (typeof expired !== "boolean") return invalid();
  return { id, targetNpub, targetName, expiresAt, expired };
}

export function parsePendingInvitations(value: unknown, brainId: string): PendingInvitation[] {
  const data = record(value);
  if (data.version !== 1 || data.brainId !== brainId || !Array.isArray(data.invitations) || data.invitations.length > 500) {
    return invalid();
  }
  const rows = data.invitations.map(invitation);
  if (new Set(rows.map(row => row.id)).size !== rows.length) return invalid();
  return rows;
}

type Dependencies = {
  read: (runtimeId: string, path: string, signal: AbortSignal) => Promise<unknown>;
  send: (runtimeId: string, path: string, change: HostedHermesChange, signal: AbortSignal) => Promise<HostedHermesChangeResult>;
};
const browser: Dependencies = { read: readHostedHermesJson, send: sendHostedHermesChange };
const base = "api/plugins/finite-brain";

function checkedBrain(brainId: string) {
  if (!BRAIN_ID.test(brainId)) throw new RosterChangeError("invalidInput");
  return brainId;
}

/** Send once. A thrown grant or access error means the agent was not
 * reached; anything else after sending is an unknown outcome. */
async function change(dependencies: Dependencies, runtimeId: string, path: string, body: HostedHermesChange, signal: AbortSignal) {
  let result: HostedHermesChangeResult;
  try {
    result = await dependencies.send(runtimeId, path, body, signal);
  } catch (error) {
    if (signal.aborted) throw error;
    if (error instanceof HostedHermesStatusError && error.kind === "access") throw new RosterChangeError("access", error.message);
    throw new RosterChangeError("unknown");
  }
  if (result.status >= 200 && result.status < 300) return result.body;
  const code = (result.body as { error?: unknown } | null)?.error;
  if (typeof code === "string" && ERROR_CODES.has(code as ChangeErrorCode)) throw new RosterChangeError(code as ChangeErrorCode);
  // Native auth refused it before the plugin ran; nothing changed.
  if (result.status === 401) throw new RosterChangeError("access", "Access to this agent is no longer available.");
  if (result.status === 404 || result.status === 405) throw new RosterChangeError("unavailable");
  throw new RosterChangeError("unknown");
}

export async function readPendingInvitations(runtimeId: string, brainId: string, signal: AbortSignal, dependencies = browser) {
  return parsePendingInvitations(await dependencies.read(runtimeId, `${base}/invitations/${checkedBrain(brainId)}`, signal), brainId);
}

/** Returns the created invitation, or null when the agent created it but
 * returned no readable receipt (the refreshed list shows it). */
export async function inviteToBrain(runtimeId: string, brainId: string, input: string, signal: AbortSignal, dependencies = browser) {
  const target = inviteTarget(input);
  if (!target) throw new RosterChangeError("invalidInput");
  const body = record(await change(dependencies, runtimeId, `${base}/invitations/${checkedBrain(brainId)}`,
    { method: "POST", body: { target } }, signal));
  if (body.version !== 1) throw new RosterChangeError("unknown");
  try { return body.invitation == null ? null : invitation(body.invitation); } catch { return null; }
}

export async function revokeInvitation(runtimeId: string, brainId: string, invitationId: string, signal: AbortSignal, dependencies = browser) {
  if (!INVITATION_ID.test(invitationId)) throw new RosterChangeError("invalidInput");
  await change(dependencies, runtimeId, `${base}/invitations/${checkedBrain(brainId)}/${invitationId}`, { method: "DELETE" }, signal);
}

function job(value: unknown): { id: string | null; state: RemovalState } {
  const data = record(value);
  if (data.version !== 1) return invalid();
  const item = record(data.job);
  if (!REMOVAL_STATES.has(item.state as RemovalState)) return invalid();
  const id = typeof item.id === "string" && JOB_ID.test(item.id) ? item.id : null;
  return { id, state: item.state as RemovalState };
}

export async function startRemoval(runtimeId: string, brainId: string, npub: string, signal: AbortSignal, dependencies = browser) {
  if (!NPUB.test(npub)) throw new RosterChangeError("invalidInput");
  const started = await change(dependencies, runtimeId, `${base}/removals/${checkedBrain(brainId)}`,
    { method: "POST", body: { npub } }, signal);
  let parsed;
  try { parsed = job(started); } catch { throw new RosterChangeError("unknown"); }
  if (!parsed.id || parsed.state !== "running") throw new RosterChangeError("unknown");
  return parsed.id;
}

export async function readRemoval(runtimeId: string, brainId: string, jobId: string, signal: AbortSignal, dependencies = browser) {
  if (!JOB_ID.test(jobId)) return "unknown" as const;
  return job(await dependencies.read(runtimeId, `${base}/removals/${checkedBrain(brainId)}/${jobId}`, signal)).state;
}

type Sleep = (milliseconds: number, signal: AbortSignal) => Promise<void>;
const sleep: Sleep = (milliseconds, signal) => new Promise((resolve, reject) => {
  const timer = setTimeout(resolve, milliseconds);
  signal.addEventListener("abort", () => { clearTimeout(timer); reject(signal.reason); }, { once: true });
});

/** Poll a removal job until it finishes. Status reads are reads, so a failed
 * poll is tried again; a job the agent no longer knows (a restart) or one
 * that outlasts the plugin's own bounds reads as unknown. */
export async function waitForRemoval(
  runtimeId: string, brainId: string, jobId: string, signal: AbortSignal,
  { dependencies = browser, wait = sleep, intervalMs = 2000, maxPolls = 330, maxFailures = 5 } = {},
): Promise<Exclude<RemovalState, "running">> {
  let failures = 0;
  for (let poll = 0; poll < maxPolls; poll++) {
    await wait(intervalMs, signal);
    try {
      const state = await readRemoval(runtimeId, brainId, jobId, signal, dependencies);
      failures = 0;
      if (state !== "running") return state;
    } catch (error) {
      if (signal.aborted) throw error;
      if (++failures >= maxFailures) return "unknown";
    }
  }
  return "unknown";
}

/** Plain copy for each outcome. `who` names the person or target. */
export function changeMessage(code: ChangeErrorCode, agentName: string, action: "invite" | "revoke" | "remove", who: string) {
  switch (code) {
    case "busy": return `${agentName} is already making a Brain change. Try again when it finishes.`;
    case "notAdmin": return `${agentName} could not confirm it is an admin of this Brain.`;
    case "invalidInput": return action === "invite" ? "Enter an npub or a public name such as name@finite.vip."
      : "That request was not accepted. Refresh and try again.";
    case "notPending": return "That invitation is no longer pending. Refresh to see the current list.";
    case "notRemovable": return `${who} is not a Member or Admin of this Brain now. Refresh to check.`;
    case "ownKey": return `${agentName} cannot remove itself here.`;
    case "unavailable": return `${agentName} could not check this Brain right now. Try again.`;
    case "access": return "Access to this agent is no longer available.";
    case "unknown": return `${agentName} did not confirm the result. Refresh to check.`;
    case "failed": return action === "invite" ? `Could not invite ${who}. Ask ${agentName} in Chat for details.`
      : action === "revoke" ? `Could not revoke the invitation for ${who}. Ask ${agentName} in Chat for details.`
        : `Could not remove ${who}. Ask ${agentName} in Chat for details.`;
  }
}

export function removalMessage(state: Exclude<RemovalState, "running">, agentName: string, who: string) {
  switch (state) {
    case "removed": return `${who} was removed. Their Folders were re-keyed.`;
    case "demotedNotRemoved": return `${who} is now a Member and was not removed. Ask ${agentName} in Chat for details.`;
    case "failed": return `Could not remove ${who}. Ask ${agentName} in Chat for details.`;
    case "unknown": return `Could not confirm whether ${who} was removed. Refresh to check.`;
  }
}
