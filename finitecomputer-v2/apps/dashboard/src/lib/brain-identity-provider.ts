import { createHash, createHmac, randomUUID, timingSafeEqual } from "node:crypto";

import type { BrainIdentityProviderRequest } from "@/lib/hosted-web-device";
import { browserVisibleRequestOrigin } from "@/lib/http-headers";

const BRAIN_IDENTITY_PROVIDER_VERSION = "finite-brain-identity-provider-v1";
const BRAIN_CLIENT_CAPABILITY_VERSION = "finite-brain-client-v1";
const BRAIN_CLIENT_CAPABILITY_TTL_SECONDS = 8 * 60 * 60;
const BRAIN_SESSION_PROOF_VERSION = "finite-brain-session-proof-v1";
const BRAIN_SESSION_PROOF_TTL_SECONDS = 10;
const BRAIN_IDENTITY_OPERATIONS = new Set<BrainIdentityProviderRequest["operation"]>([
  "identifyMember",
  "authorizeHttpRequest",
  "authorizeBrainEvent",
  "openGrantPayload",
  "wrapGrantPayload",
]);

export type BrainClientCapabilityAccount = {
  workosUserId: string;
  emailVerified: true;
  brainPublicOrigin: string;
};

type CurrentBrainAccount = {
  workosUserId: string | null;
  emailVerified: boolean;
};

type VerifiedBrainSessionProof = {
  workosUserId: string;
  emailVerified: true;
};

export function brainCapabilityMatchesCurrentAccount(
  capability: BrainClientCapabilityAccount,
  account: CurrentBrainAccount,
) {
  return (
    account.emailVerified &&
    Boolean(account.workosUserId) &&
    account.workosUserId === capability.workosUserId
  );
}

export function brainIdentityRequestHash(bodyText: string) {
  return createHash("sha256").update(bodyText).digest("hex");
}

export function issueBrainSessionProof(
  secret: string,
  workosUserId: string,
  requestHash: string,
  nowUnixSeconds = Math.floor(Date.now() / 1000),
  nonce = randomUUID(),
) {
  if (!secret || !workosUserId || !/^[0-9a-f]{64}$/u.test(requestHash)) {
    throw new Error("Brain session proof inputs are required.");
  }
  const claims = Buffer.from(
    JSON.stringify({
      version: BRAIN_SESSION_PROOF_VERSION,
      workosUserId,
      requestHash,
      expiresAt: nowUnixSeconds + BRAIN_SESSION_PROOF_TTL_SECONDS,
      nonce,
    }),
  ).toString("base64url");
  return `${claims}.${brainSessionProofMac(secret, claims)}`;
}

export function verifyBrainSessionProof(
  token: string | null,
  secret: string,
  requestHash: string,
  nowUnixSeconds = Math.floor(Date.now() / 1000),
): VerifiedBrainSessionProof | null {
  if (!token || !secret || !/^[0-9a-f]{64}$/u.test(requestHash)) return null;
  const [claims, signature, extra] = token.split(".");
  if (!claims || !signature || extra) return null;
  const expected = Buffer.from(brainSessionProofMac(secret, claims), "base64url");
  const actual = Buffer.from(signature, "base64url");
  if (actual.length !== expected.length || !timingSafeEqual(actual, expected)) return null;
  try {
    const value = JSON.parse(Buffer.from(claims, "base64url").toString("utf8")) as Record<
      string,
      unknown
    >;
    if (
      value.version !== BRAIN_SESSION_PROOF_VERSION ||
      typeof value.workosUserId !== "string" ||
      !value.workosUserId ||
      value.requestHash !== requestHash ||
      typeof value.expiresAt !== "number" ||
      !Number.isSafeInteger(value.expiresAt) ||
      value.expiresAt < nowUnixSeconds ||
      typeof value.nonce !== "string" ||
      !value.nonce
    ) {
      return null;
    }
    return { workosUserId: value.workosUserId, emailVerified: true };
  } catch {
    return null;
  }
}

export function officialBrainFrameNavigation(
  requestUrl: string,
  headers: Pick<Headers, "get">,
) {
  return officialBrainFrameParentOrigin(requestUrl, headers) !== null;
}

export function officialBrainFrameOrigins(
  requestUrl: string,
  headers: Pick<Headers, "get">,
  brainPublicOrigin: string | undefined,
) {
  const parentOrigin = officialBrainFrameParentOrigin(requestUrl, headers);
  if (!parentOrigin || !brainPublicOrigin || !exactHttpOrigin(brainPublicOrigin)) {
    return null;
  }
  return { parentOrigin, brainPublicOrigin };
}

export function officialBrainFrameParentOrigin(
  requestUrl: string,
  headers: Pick<Headers, "get">,
) {
  const referer = headers.get("referer");
  const publicOrigin = browserVisibleRequestOrigin({ headers, url: requestUrl });
  if (
    !referer ||
    !publicOrigin ||
    headers.get("sec-fetch-dest") !== "iframe" ||
    headers.get("sec-fetch-mode") !== "navigate" ||
    headers.get("sec-fetch-site") !== "same-origin"
  ) {
    return null;
  }
  try {
    const source = new URL(referer);
    return (
      source.origin === publicOrigin &&
      /^\/dashboard\/machines\/[^/]+\/brain$/u.test(source.pathname)
        ? source.origin
        : null
    );
  } catch {
    return null;
  }
}

export function issueBrainClientCapability(
  secret: string,
  workosUserId: string,
  brainPublicOrigin: string,
  nowUnixSeconds = Math.floor(Date.now() / 1000),
  nonce = randomUUID(),
) {
  if (!secret || !workosUserId || !exactHttpOrigin(brainPublicOrigin)) {
    throw new Error("Brain client capability inputs are required.");
  }
  const claims = Buffer.from(
    JSON.stringify({
      version: BRAIN_CLIENT_CAPABILITY_VERSION,
      workosUserId,
      brainPublicOrigin,
      expiresAt: nowUnixSeconds + BRAIN_CLIENT_CAPABILITY_TTL_SECONDS,
      nonce,
    }),
  ).toString("base64url");
  return `${claims}.${brainCapabilityMac(secret, claims)}`;
}

export function verifyBrainClientCapability(
  token: string | null,
  secret: string,
  nowUnixSeconds = Math.floor(Date.now() / 1000),
): BrainClientCapabilityAccount | null {
  if (!token || !secret) return null;
  const [claims, signature, extra] = token.split(".");
  if (!claims || !signature || extra) return null;
  const expected = Buffer.from(brainCapabilityMac(secret, claims), "base64url");
  const actual = Buffer.from(signature, "base64url");
  if (actual.length !== expected.length || !timingSafeEqual(actual, expected)) return null;
  try {
    const value = JSON.parse(Buffer.from(claims, "base64url").toString("utf8")) as Record<
      string,
      unknown
    >;
    if (
      value.version !== BRAIN_CLIENT_CAPABILITY_VERSION ||
      typeof value.workosUserId !== "string" ||
      !value.workosUserId ||
      typeof value.brainPublicOrigin !== "string" ||
      !exactHttpOrigin(value.brainPublicOrigin) ||
      typeof value.expiresAt !== "number" ||
      !Number.isSafeInteger(value.expiresAt) ||
      value.expiresAt < nowUnixSeconds ||
      typeof value.nonce !== "string" ||
      !value.nonce
    ) {
      return null;
    }
    return {
      workosUserId: value.workosUserId,
      emailVerified: true,
      brainPublicOrigin: value.brainPublicOrigin,
    };
  } catch {
    return null;
  }
}

function exactHttpOrigin(value: string) {
  try {
    const parsed = new URL(value);
    return (
      (parsed.protocol === "http:" || parsed.protocol === "https:") && parsed.origin === value
    );
  } catch {
    return false;
  }
}

function brainCapabilityMac(secret: string, claims: string) {
  return createHmac("sha256", secret)
    .update(`${BRAIN_CLIENT_CAPABILITY_VERSION}:${claims}`)
    .digest("base64url");
}

function brainSessionProofMac(secret: string, claims: string) {
  return createHmac("sha256", secret)
    .update(`${BRAIN_SESSION_PROOF_VERSION}:${claims}`)
    .digest("base64url");
}

export function parseBrainIdentityProviderRequest(value: unknown): BrainIdentityProviderRequest {
  if (!value || typeof value !== "object") {
    throw new Error("Brain identity request must be an object.");
  }
  const record = value as Record<string, unknown>;
  if (
    record.version !== BRAIN_IDENTITY_PROVIDER_VERSION ||
    typeof record.operation !== "string" ||
    !BRAIN_IDENTITY_OPERATIONS.has(record.operation as BrainIdentityProviderRequest["operation"]) ||
    !("input" in record)
  ) {
    throw new Error("Brain identity request uses an unsupported version or operation.");
  }
  return {
    version: BRAIN_IDENTITY_PROVIDER_VERSION,
    operation: record.operation as BrainIdentityProviderRequest["operation"],
    input: record.input,
  };
}
