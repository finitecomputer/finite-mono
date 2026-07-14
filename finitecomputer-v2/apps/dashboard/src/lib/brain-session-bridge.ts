export const BRAIN_SESSION_PROOF_REQUEST = "finite-brain-session-proof-request-v1";
export const BRAIN_SESSION_PROOF_RESPONSE = "finite-brain-session-proof-response-v1";
export const BRAIN_SESSION_ENDED = "finite-brain-session-ended-v1";

export type BrainSessionProofRequest = {
  type: typeof BRAIN_SESSION_PROOF_REQUEST;
  requestId: string;
  requestHash: string;
};

export function brainClientPath(agentNpub: string | null | undefined) {
  const candidate = agentNpub?.trim();
  if (
    !candidate ||
    !candidate.toLowerCase().startsWith("npub1") ||
    candidate.length > 256 ||
    !/^[a-z0-9]+$/iu.test(candidate)
  ) {
    return "/client";
  }
  return `/client?agentNpub=${encodeURIComponent(candidate)}`;
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

export function endEmbeddedBrainSession() {
  document.querySelectorAll<HTMLIFrameElement>("iframe[data-finite-brain-frame]").forEach((frame) => {
    frame.contentWindow?.postMessage({ type: BRAIN_SESSION_ENDED }, "*");
  });
}
