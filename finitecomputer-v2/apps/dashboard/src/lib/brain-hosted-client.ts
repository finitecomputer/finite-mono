import { createHash, randomBytes } from "node:crypto";

import { type AccountAuthContext } from "@/lib/dashboard-auth";
import {
  type BrainIdentityProviderResponse,
  type HostedDeviceConfig,
  type HostedDeviceRequestError,
  hostedDeviceBrainIdentityProvider,
} from "@/lib/hosted-web-device";

/// Server-side Brain client for the chat surface: every request is signed by
/// the account's human principal through the hosted chat device (the same
/// authority the deleted Product Client and the CLI approvals path drive).

const BRAIN_REQUEST_TIMEOUT_MS = 30_000;

export class BrainHostedClientError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "BrainHostedClientError";
    this.status = status;
  }
}

export function brainServerOrigin(value = process.env.FC_BRAIN_UPSTREAM_URL) {
  const candidate = value?.trim().replace(/\/$/u, "");
  if (!candidate) return null;
  try {
    const url = new URL(candidate);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    if (url.pathname !== "/" || url.search || url.hash) return null;
    return url.origin;
  } catch {
    return null;
  }
}

type AuthorizeHttpRequestInput = {
  method: string;
  url: string;
  bodyText: string;
  eventTemplate: {
    kind: number;
    created_at: number;
    tags: string[][];
    content: string;
  };
};

function sha256Hex(text: string) {
  return createHash("sha256").update(text).digest("hex");
}

function nonceHex() {
  return randomBytes(16).toString("hex");
}

export async function hostedSignedBrainRequest(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  brainOrigin: string,
  method: string,
  path: string,
  bodyText = ""
): Promise<unknown> {
  const url = `${brainOrigin}${path}`;
  const tags: string[][] = [
    ["u", url],
    ["method", method.toUpperCase()],
    ["nonce", nonceHex()],
  ];
  if (bodyText) tags.push(["payload", sha256Hex(bodyText)]);
  const input: AuthorizeHttpRequestInput = {
    method: method.toUpperCase(),
    url,
    bodyText,
    eventTemplate: {
      kind: 27_235,
      created_at: Math.floor(Date.now() / 1000),
      tags,
      content: "",
    },
  };
  const signed = (await hostedDeviceBrainIdentityProvider(
    config,
    account,
    { version: "finite-brain-identity-provider-v1", operation: "authorizeHttpRequest", input },
    brainOrigin
  )) as { event?: unknown };
  const eventJson = (signed as { event?: unknown }).event ?? signed;
  const authorization = `Nostr ${Buffer.from(JSON.stringify(eventJson), "utf8").toString("base64")}`;
  const response = await fetch(url, {
    method: method.toUpperCase(),
    cache: "no-store",
    headers: {
      authorization,
      ...(bodyText ? { "content-type": "application/json" } : {}),
    },
    body: bodyText || undefined,
    signal: AbortSignal.timeout(BRAIN_REQUEST_TIMEOUT_MS),
  });
  const text = await response.text();
  if (!response.ok) {
    throw new BrainHostedClientError(
      text || `Brain ${method} ${path} returned HTTP ${response.status}`,
      response.status
    );
  }
  if (!text.trim()) return { status: "ok" };
  try {
    return JSON.parse(text);
  } catch {
    throw new BrainHostedClientError(
      `Brain ${method} ${path} returned a non-JSON body`,
      502
    );
  }
}

export async function hostedSignBrainApproval(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  brainOrigin: string,
  input: {
    brainId: string;
    action: string;
    planId?: string | null;
    targetNpubs?: string[];
    nonce: string;
    expiresAt: number;
  }
): Promise<string> {
  const signed = (await hostedDeviceBrainIdentityProvider(
    config,
    account,
    { version: "finite-brain-identity-provider-v1", operation: "approveBrainAction", input },
    brainOrigin
  )) as { event?: unknown };
  const eventJson = (signed as { event?: unknown }).event ?? signed;
  return JSON.stringify(eventJson);
}

export type { HostedDeviceRequestError, BrainIdentityProviderResponse };
