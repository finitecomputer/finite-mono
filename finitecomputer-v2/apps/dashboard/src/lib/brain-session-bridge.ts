export const BRAIN_SESSION_PROOF_REQUEST = "finite-brain-session-proof-request-v1";
export const BRAIN_SESSION_PROOF_RESPONSE = "finite-brain-session-proof-response-v1";
export const BRAIN_SESSION_ENDED = "finite-brain-session-ended-v1";
export const BRAIN_PERSONAL_AGENT_CONFIRMATION_REQUEST =
  "finite-brain-personal-agent-confirmation-request-v1";
export const BRAIN_PERSONAL_AGENT_CONFIRMATION_RESPONSE =
  "finite-brain-personal-agent-confirmation-response-v1";
export const BRAIN_FRAME_SANDBOX =
  "allow-downloads allow-forms allow-modals allow-scripts";

export type BrainSessionProofRequest = {
  type: typeof BRAIN_SESSION_PROOF_REQUEST;
  requestId: string;
  requestHash: string;
};

export type BrainPersonalAgentConfirmationRequest = {
  type: typeof BRAIN_PERSONAL_AGENT_CONFIRMATION_REQUEST;
  requestId: string;
  identity: string;
};

export type BrainAgentIdentityHint = {
  email?: string | null;
  name?: string | null;
  npub?: string | null;
  brainId?: string | null;
};

export function brainClientPath(identity: BrainAgentIdentityHint | null | undefined) {
  if (!identity) return "/client";
  const email = boundedAgentEmail(identity.email);
  const name = boundedAgentName(identity.name);
  const npub = boundedAgentNpub(identity.npub);
  const brainId = boundedBrainId(identity.brainId);
  if (!email && !npub && !brainId) return "/client";

  const query = new URLSearchParams();
  if (email) query.set("agentEmail", email);
  if (name) query.set("agentName", name);
  if (!email && npub) query.set("agentNpub", npub);
  if (brainId) query.set("brainId", brainId);
  return `/client?${query.toString()}`;
}

export function brainMachinePath(machineId: string, brainId?: string | null) {
  const path = `/dashboard/machines/${encodeURIComponent(machineId)}/brain`;
  const target = boundedBrainId(brainId);
  return target ? `${path}?brainId=${encodeURIComponent(target)}` : path;
}

function boundedBrainId(value: string | null | undefined) {
  const candidate = value?.trim();
  return candidate && /^[a-z0-9][a-z0-9_-]{0,127}$/u.test(candidate) ? candidate : null;
}

function boundedAgentEmail(value: string | null | undefined) {
  const candidate = value?.trim().toLowerCase();
  if (
    !candidate ||
    candidate.length > 254 ||
    !/^[a-z0-9._-]+@[a-z0-9.-]+$/u.test(candidate)
  ) {
    return null;
  }
  return candidate;
}

function boundedAgentName(value: string | null | undefined) {
  const candidate = value?.trim();
  if (!candidate || candidate.length > 80 || /[\u0000-\u001f\u007f]/u.test(candidate)) {
    return null;
  }
  return candidate;
}

function boundedAgentNpub(value: string | null | undefined) {
  const candidate = value?.trim();
  if (
    !candidate ||
    !candidate.toLowerCase().startsWith("npub1") ||
    candidate.length > 256 ||
    !/^[a-z0-9]+$/iu.test(candidate)
  ) {
    return null;
  }
  return candidate;
}

export function parseBrainSessionProofRequest(value: unknown): BrainSessionProofRequest | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  if (
    record.type !== BRAIN_SESSION_PROOF_REQUEST ||
    typeof record.requestId !== "string" ||
    !/^[0-9a-f]{32}$/u.test(record.requestId) ||
    typeof record.requestHash !== "string" ||
    !/^[0-9a-f]{64}$/u.test(record.requestHash)
  ) {
    return null;
  }
  return {
    type: BRAIN_SESSION_PROOF_REQUEST,
    requestId: record.requestId,
    requestHash: record.requestHash,
  };
}

export function parseBrainPersonalAgentConfirmationRequest(
  value: unknown,
  expectedIdentity: string | null | undefined,
): BrainPersonalAgentConfirmationRequest | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  const expected = expectedIdentity?.trim().toLowerCase();
  if (
    !expected ||
    record.type !== BRAIN_PERSONAL_AGENT_CONFIRMATION_REQUEST ||
    typeof record.requestId !== "string" ||
    !/^[0-9a-f]{32}$/u.test(record.requestId) ||
    typeof record.identity !== "string" ||
    record.identity.trim().toLowerCase() !== expected
  ) {
    return null;
  }
  return {
    type: BRAIN_PERSONAL_AGENT_CONFIRMATION_REQUEST,
    requestId: record.requestId,
    identity: expected,
  };
}

export function endEmbeddedBrainSession() {
  document.querySelectorAll<HTMLIFrameElement>("iframe[data-finite-brain-frame]").forEach((frame) => {
    frame.contentWindow?.postMessage({ type: BRAIN_SESSION_ENDED }, "*");
  });
}
