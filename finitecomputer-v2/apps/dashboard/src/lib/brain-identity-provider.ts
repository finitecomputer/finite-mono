import type { BrainIdentityProviderRequest } from "@/lib/hosted-web-device";

const BRAIN_IDENTITY_PROVIDER_VERSION = "finite-brain-identity-provider-v1";
const BRAIN_IDENTITY_OPERATIONS = new Set<BrainIdentityProviderRequest["operation"]>([
  "identifyMember",
  "authorizeHttpRequest",
  "authorizeBrainEvent",
  "openGrantPayload",
  "wrapGrantPayload",
]);

export function officialBrainClientRequest(requestUrl: string, referer: string | null) {
  if (!referer) return false;
  try {
    const request = new URL(requestUrl);
    const source = new URL(referer);
    return source.origin === request.origin && source.pathname === "/client";
  } catch {
    return false;
  }
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
