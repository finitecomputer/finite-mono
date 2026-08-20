/// The `metadata.approve` convention carried on chat messages (the message
/// payload's metadata map, surfaced as `metadata_json`). Reference-only by
/// contract: the envelope names a server-held approval request and never
/// carries approval payload fields — the brain server's stored request is
/// the only thing a signer ever binds.
///
/// Two ends write it: the agent's delivery (the question) and the human's
/// reply (the record of their choice). Both are plain chat messages; the
/// cryptographic authority lives in the finite-brain-approval-v1 artifact
/// the approve route signs, never here.

export type BrainApprovalReference = {
  brainId: string;
  requestId: string;
};

export type BrainApproveQuestion = {
  service: "brain";
  requests: BrainApprovalReference[];
};

export type BrainApproveChoice = "approved" | "denied";

export type BrainApproveResponse = {
  service: "brain";
  choice: BrainApproveChoice;
  requests: BrainApprovalReference[];
  artifactId?: string;
};

const MAX_REFERENCES = 16;

function referenceFromValue(value: unknown): BrainApprovalReference | null {
  if (typeof value !== "object" || value === null) return null;
  const brainId = (value as Record<string, unknown>).brainId;
  const requestId = (value as Record<string, unknown>).requestId;
  if (typeof brainId !== "string" || typeof requestId !== "string") return null;
  if (!brainId || !requestId) return null;
  if (!/^[a-z0-9][a-z0-9_-]{0,127}$/u.test(brainId)) return null;
  if (requestId.length > 128) return null;
  return { brainId, requestId };
}

function referencesFromValue(value: unknown): BrainApprovalReference[] {
  if (!Array.isArray(value)) return [];
  return value
    .map(referenceFromValue)
    .filter((reference): reference is BrainApprovalReference => reference !== null)
    .slice(0, MAX_REFERENCES);
}

function approveValue(message: { metadata_json?: string }): unknown {
  const encoded = message.metadata_json?.trim();
  if (!encoded) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(encoded);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) return null;
  const approve = (parsed as Record<string, unknown>).approve;
  return typeof approve === "object" && approve !== null ? approve : null;
}

/// The envelope decoded once from `metadata_json`: both ends discriminated
/// from a single parse. At most one side is non-null — the question never
/// carries a string `choice`; the response always requires one.
export type BrainApproveEnvelope = {
  question: BrainApproveQuestion | null;
  response: BrainApproveResponse | null;
};

/// One parse, both discriminations — for callers walking many messages (the
/// chat surface decodes each message once per state version, then reads both
/// sides off the map). Behaves exactly like the accessor pair below: both
/// fail-closed branches are preserved (the question rejects a string
/// `choice`; the response requires it, approved or denied).
export function decodeApproveEnvelope(
  message: { metadata_json?: string },
): BrainApproveEnvelope {
  const envelope: BrainApproveEnvelope = { question: null, response: null };
  const approve = approveValue(message);
  if (approve === null) return envelope;
  const record = approve as Record<string, unknown>;
  if (record.service !== "brain") return envelope;
  const requests = referencesFromValue(record.requests);
  if (requests.length === 0) return envelope;
  if (typeof record.choice === "string") {
    if (record.choice !== "approved" && record.choice !== "denied") return envelope;
    const artifactId = record.artifactId;
    envelope.response = {
      service: "brain",
      choice: record.choice,
      requests,
      artifactId: typeof artifactId === "string" && artifactId ? artifactId : undefined,
    };
  } else {
    envelope.question = { service: "brain", requests };
  }
  return envelope;
}

/// The agent's question: which approval requests this delivery asks about.
/// Returns null unless the envelope is well-formed and non-empty.
export function parseApproveQuestion(
  message: { metadata_json?: string },
): BrainApproveQuestion | null {
  const approve = approveValue(message);
  if (approve === null) return null;
  const record = approve as Record<string, unknown>;
  if (record.service !== "brain") return null;
  if (typeof record.choice === "string") return null;
  const requests = referencesFromValue(record.requests);
  if (requests.length === 0) return null;
  return { service: "brain", requests };
}

/// The human's reply: their recorded choice for the named requests.
export function parseApproveResponse(
  message: { metadata_json?: string },
): BrainApproveResponse | null {
  const approve = approveValue(message);
  if (approve === null) return null;
  const record = approve as Record<string, unknown>;
  if (record.service !== "brain") return null;
  if (record.choice !== "approved" && record.choice !== "denied") return null;
  const requests = referencesFromValue(record.requests);
  if (requests.length === 0) return null;
  const artifactId = record.artifactId;
  return {
    service: "brain",
    choice: record.choice,
    requests,
    artifactId: typeof artifactId === "string" && artifactId ? artifactId : undefined,
  };
}

/// Build the reply envelope for a choice, serialized for `metadata_json`.
export function approveResponseMetadata(
  choice: BrainApproveChoice,
  requests: BrainApprovalReference[],
  artifactId?: string,
): string {
  const response: BrainApproveResponse = {
    service: "brain",
    choice,
    requests: requests.slice(0, MAX_REFERENCES),
  };
  if (artifactId) response.artifactId = artifactId;
  return JSON.stringify({ approve: response });
}
