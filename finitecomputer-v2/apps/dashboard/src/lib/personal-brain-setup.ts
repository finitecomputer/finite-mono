import type { BrainRow } from "./agent-product-inventory";

/// Personal Brain setup from the dashboard Brain page. The person signs the
/// creation request through the Hosted Device; the Agent signs a short-lived
/// consent through its own runtime. The Brain server checks both signatures
/// against its own records, so neither side can name the other alone.
/// Shared by the browser flow and the server route; no I/O here.

export const NPUB = /^npub1[023456789acdefghjklmnpqrstuvwxyz]{58}$/u;
export const PERSONAL_BRAIN_ID = /^personal-[0-9a-f]{16}$/u;
export const MAX_SETUP_BODY_BYTES = 16 * 1024;
const BRAIN_ID = /^[A-Za-z0-9_-]{1,128}$/u;
const MAX_CONSENT_TAGS = 32;
const MAX_CONSENT_CONTENT = 8192;

export type PersonalAgentConsentEvent = {
  id: string;
  pubkey: string;
  created_at: number;
  kind: number;
  tags: string[][];
  content: string;
  sig: string;
};

export type PersonalBrainSetupRequest = {
  agentNpub: string;
  brainId: string;
  consent: PersonalAgentConsentEvent;
};

function plainRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null ? (value as Record<string, unknown>) : null;
}

function hex(value: unknown, length: number): value is string {
  return typeof value === "string" && value.length === length && /^[0-9a-f]+$/u.test(value);
}

export function isNpub(value: unknown): value is string {
  return typeof value === "string" && NPUB.test(value);
}

/// The Agent-signed consent event, reduced to the signed Nostr fields. The
/// Brain server verifies the signature; this only bounds the shape.
export function parseConsentEvent(value: unknown): PersonalAgentConsentEvent | null {
  const event = plainRecord(value);
  if (!event) return null;
  const { id, pubkey, created_at, kind, tags, content, sig } = event;
  if (
    !hex(id, 64) || !hex(pubkey, 64) || !hex(sig, 128) ||
    typeof content !== "string" || !content.trim() || content.length > MAX_CONSENT_CONTENT ||
    typeof created_at !== "number" || !Number.isSafeInteger(created_at) || created_at < 0 ||
    typeof kind !== "number" || !Number.isSafeInteger(kind) || kind < 0 ||
    !Array.isArray(tags) || tags.length > MAX_CONSENT_TAGS ||
    !tags.every((tag) => Array.isArray(tag) && tag.every((item) => typeof item === "string"))
  ) {
    return null;
  }
  return { id, pubkey, created_at, kind, tags: tags as string[][], content, sig };
}

/// The POST body the browser sends and the route accepts.
export function parsePersonalBrainSetupRequest(value: unknown): PersonalBrainSetupRequest | null {
  const data = plainRecord(value);
  if (!data || !isNpub(data.agentNpub)) return null;
  const brainId = data.brainId;
  if (typeof brainId !== "string" || !PERSONAL_BRAIN_ID.test(brainId)) return null;
  const consent = parseConsentEvent(data.consent);
  return consent ? { agentNpub: data.agentNpub, brainId, consent } : null;
}

/// The Agent's consent route response (version 1). It must name the owner the
/// browser asked about, and an Agent can never consent for itself.
export function parsePersonalAgentConsent(value: unknown, ownerNpub: string): PersonalBrainSetupRequest | null {
  const data = plainRecord(value);
  if (!data || data.version !== 1 || !isNpub(ownerNpub) || data.ownerNpub !== ownerNpub) return null;
  const setup = parsePersonalBrainSetupRequest(data);
  return setup && setup.agentNpub !== ownerNpub ? setup : null;
}

export function parseOwnerNpub(value: unknown): string | null {
  const ownerNpub = plainRecord(value)?.ownerNpub;
  return isNpub(ownerNpub) ? ownerNpub : null;
}

/// The owner-signed Brain creation body.
export function personalBrainCreateBody({ agentNpub, brainId, consent }: PersonalBrainSetupRequest) {
  return JSON.stringify({
    brainId,
    kind: "personal",
    name: "Personal Brain",
    personalAgentNpub: agentNpub,
    personalAgentConsent: consent,
  });
}

/// The dashboard response for a Brain refusal other than a conflict. A 4xx
/// will refuse the same request again, so it does not invite a retry.
export function brainRefusal(status: number) {
  return status >= 400 && status < 500
    ? "Brain refused this Personal Brain setup."
    : "Brain could not set up your Personal Brain. Try again.";
}

/// Plain wording for the Brain server's Personal Brain conflicts.
export function personalBrainConflictMessage(brainMessage: string) {
  if (brainMessage.includes("already the personal agent of another personal brain")) {
    return "This agent is already the Personal Agent of another Personal Brain.";
  }
  if (brainMessage.includes("already has a different personal agent")) {
    return "Your Personal Brain already has a different Personal Agent.";
  }
  if (brainMessage.includes("already has a personal brain") || brainMessage.includes("already exists")) {
    return "You already have a Personal Brain.";
  }
  return "Your Personal Brain could not be set up. Refresh and try again.";
}

export function createdPersonalBrainId(result: unknown, requested: string) {
  const brainId = plainRecord(result)?.brainId;
  return typeof brainId === "string" && BRAIN_ID.test(brainId) ? brainId : requested;
}

/// Offer setup only for a loaded inventory where this Agent holds no Personal
/// Brain of its own. An unavailable inventory proves nothing either way.
export function needsPersonalBrainSetup(brains: readonly Pick<BrainRow, "kind" | "role" | "pending">[] | null | undefined) {
  if (!brains) return false;
  return !brains.some(
    (brain) => !brain.pending && brain.kind === "Personal" && (brain.role === "Personal agent" || brain.role === "Owner"),
  );
}
