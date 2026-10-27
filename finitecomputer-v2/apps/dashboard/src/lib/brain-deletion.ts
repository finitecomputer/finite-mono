import { randomBytes } from "node:crypto";

import { type AccountAuthContext } from "@/lib/dashboard-auth";
import {
  BrainHostedClientError,
  brainPublicOrigin,
  hostedSignedBrainRequest,
} from "@/lib/brain-hosted-client";
import {
  BRAIN_ID_PATTERN,
  type BrainDeletionPreview,
  type BrainDeletionResult,
  InvalidBrainResponse,
  STATE_DIGEST_PATTERN,
  parseBrainDeletionPreview,
  parseBrainDeletionResult,
} from "@/lib/brain-deletion-view";
import { type HostedDeviceConfig, hostedDeviceBrainIdentityProvider } from "@/lib/hosted-web-device";

/// Organization Brain deletion for the dashboard. Every step is signed by the
/// account's hosted Brain key through the hosted device, the same authority
/// `fbrain brain delete` uses: preview, then a `delete-brain` admin
/// access-change event whose note is the preview's state digest. The Brain
/// server rechecks admin standing and that digest in the deleting transaction.

const ADMIN_ACCESS_CHANGE_KIND = 30_078;

/// The signed calls deletion needs; tests substitute their own.
export type BrainDeletionTransport = {
  signedRequest: (method: string, path: string, bodyText?: string) => Promise<unknown>;
  identify: () => Promise<{ npub: string }>;
  signBrainEvent: (eventTemplate: BrainEventTemplate) => Promise<unknown>;
};

export type BrainEventTemplate = {
  kind: number;
  created_at: number;
  tags: string[][];
  content: string;
};

/// RFC 3339 at whole seconds, as the Brain server formats a signed event's
/// `created_at` (no fractional seconds).
function rfc3339Seconds(unixSeconds: number) {
  return new Date(unixSeconds * 1000).toISOString().replace(/\.\d{3}Z$/u, "Z");
}

/// One unsigned delete-brain event with the preview digest as its note.
/// Content and tags must equal
/// `AdminAccessChangePayload::canonical_json` and `admin_access_change_tags`
/// in finite-brain-core byte for byte, or the server refuses the signature.
export function brainDeletionEventTemplate(input: {
  brainId: string;
  adminNpub: string;
  stateDigest: string;
  changeId: string;
  createdAt: number;
}): BrainEventTemplate {
  if (!BRAIN_ID_PATTERN.test(input.brainId) || !STATE_DIGEST_PATTERN.test(input.stateDigest)) {
    throw new Error("Brain deletion needs a Brain id and a preview state digest.");
  }
  const content = [
    `"version":${JSON.stringify("finite-brain-admin-access-change-v1")}`,
    `"brainId":${JSON.stringify(input.brainId)}`,
    `"changeId":${JSON.stringify(input.changeId)}`,
    `"action":"delete-brain"`,
    `"adminNpub":${JSON.stringify(input.adminNpub)}`,
    `"note":${JSON.stringify(input.stateDigest)}`,
    `"createdAt":${JSON.stringify(rfc3339Seconds(input.createdAt))}`,
  ].join(",");
  return {
    kind: ADMIN_ACCESS_CHANGE_KIND,
    created_at: input.createdAt,
    tags: [
      ["d", `finite-brain-admin-access-change:${input.brainId}:${input.changeId}`],
      ["brain", input.brainId],
      ["action", "delete-brain"],
    ],
    content: `{${content}}`,
  };
}

export async function previewBrainDeletion(
  transport: BrainDeletionTransport,
  brainId: string
): Promise<BrainDeletionPreview> {
  return parseBrainDeletionPreview(
    await transport.signedRequest("GET", `/v1/brains/${encodeURIComponent(brainId)}/deletion-preview`)
  );
}

/// Sign and submit deletion of exactly the previewed state. A transport
/// failure leaves the outcome unknown, so the identical signed deletion is
/// sent once more: the server either deletes or answers the accepted receipt.
/// A Brain server answer is final and never retried.
export async function deleteBrainExactly(
  transport: BrainDeletionTransport,
  input: { brainId: string; stateDigest: string },
  nowUnixSeconds = Math.floor(Date.now() / 1000)
): Promise<BrainDeletionResult> {
  const { npub } = await transport.identify();
  const template = brainDeletionEventTemplate({
    brainId: input.brainId,
    adminNpub: npub,
    stateDigest: input.stateDigest,
    changeId: `delete-brain-${randomBytes(16).toString("hex")}`,
    createdAt: nowUnixSeconds,
  });
  const deletionEvent = await transport.signBrainEvent(template);
  const bodyText = JSON.stringify({ deletionEvent });
  const path = `/v1/brains/${encodeURIComponent(input.brainId)}`;
  let response: unknown;
  try {
    response = await transport.signedRequest("DELETE", path, bodyText);
  } catch (error) {
    if (error instanceof BrainHostedClientError) throw error;
    response = await transport.signedRequest("DELETE", path, bodyText);
  }
  return parseBrainDeletionResult(response);
}

export function hostedBrainDeletionTransport(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  brainOrigin: string
): BrainDeletionTransport {
  const identityOrigin = brainPublicOrigin() ?? brainOrigin;
  const provider = (operation: "identifyMember" | "authorizeBrainEvent", input: unknown) =>
    hostedDeviceBrainIdentityProvider(
      config,
      account,
      { version: "finite-brain-identity-provider-v1", operation, input },
      identityOrigin
    );
  return {
    signedRequest: (method, path, bodyText) =>
      hostedSignedBrainRequest(config, account, brainOrigin, method, path, bodyText),
    identify: async () => {
      const identity = await provider("identifyMember", null);
      if (typeof identity.npub !== "string" || !identity.npub.startsWith("npub1")) {
        throw new InvalidBrainResponse("The hosted Brain key could not be identified.");
      }
      return { npub: identity.npub };
    },
    signBrainEvent: async (eventTemplate) => {
      const signed = await provider("authorizeBrainEvent", {
        intent: "brain-access-change",
        eventTemplate,
      });
      return signed.event ?? signed;
    },
  };
}

/// What the dashboard tells the account after a refused request.
export function brainDeletionFailure(error: unknown): { status: number; message: string } {
  if (error instanceof BrainHostedClientError) {
    if ([403, 404, 409, 410].includes(error.status)) {
      return { status: error.status, message: error.message };
    }
    if (error.status === 400) return { status: 400, message: error.message };
    return { status: 502, message: "Brain couldn't complete the deletion request right now." };
  }
  if (error instanceof InvalidBrainResponse) return { status: 502, message: error.message };
  return { status: 502, message: "Brain deletion couldn't be signed right now." };
}
