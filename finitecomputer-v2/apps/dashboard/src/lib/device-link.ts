import { getAccountAuthContext } from "@/lib/dashboard-auth";
import {
  HostedDeviceRequestError,
  hostedDeviceApproveLink,
  hostedDeviceConfig,
  hostedDeviceLinkStatus,
  hostedDeviceState,
  type HostedDeviceLinkRequest,
  type HostedDeviceLinkResponse,
} from "@/lib/hosted-web-device";

const MAX_DEVICE_LINK_TOKEN_BYTES = 256;
const NOSTR_ACCOUNT_ID_PATTERN = /^[0-9a-f]{64}$/u;

export const MAX_DEVICE_LINK_REQUEST_BYTES = 4 * 1024;

export type HostedWebAccountBinding = {
  account_id: string;
};

export class DeviceLinkError extends Error {
  constructor(
    message: string,
    readonly status: number
  ) {
    super(message);
    this.name = "DeviceLinkError";
  }
}

export function parseDeviceLinkRequest(value: unknown): HostedDeviceLinkRequest {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new DeviceLinkError("This device-link request is incomplete.", 400);
  }

  const record = value as Record<string, unknown>;
  const keys = Object.keys(record).sort();
  if (
    keys.length !== 2 ||
    keys[0] !== "link_session_id" ||
    keys[1] !== "target_device_id"
  ) {
    throw new DeviceLinkError("This device-link request is invalid.", 400);
  }

  const linkSessionId = deviceLinkToken("link session", record.link_session_id);
  const targetDeviceId = deviceLinkToken("Device", record.target_device_id);
  if (targetDeviceId === "hosted-web") {
    throw new DeviceLinkError(
      "The new Device must be distinct from this web Device.",
      400
    );
  }

  return {
    link_session_id: linkSessionId,
    target_device_id: targetDeviceId,
  };
}

export async function parseDeviceLinkJsonRequest(
  request: Request
): Promise<HostedDeviceLinkRequest> {
  const mediaType = (request.headers.get("content-type") ?? "")
    .split(";", 1)[0]
    .trim()
    .toLowerCase();
  if (mediaType !== "application/json") {
    throw new DeviceLinkError("Device-link requests must use JSON.", 415);
  }

  const contentLength = request.headers.get("content-length");
  if (contentLength !== null) {
    const declaredLength = Number(contentLength);
    if (!Number.isSafeInteger(declaredLength) || declaredLength < 0) {
      throw new DeviceLinkError("This device-link request is invalid.", 400);
    }
    if (declaredLength > MAX_DEVICE_LINK_REQUEST_BYTES) {
      throw new DeviceLinkError("Device-link request is too large.", 413);
    }
  }

  if (!request.body) {
    throw new DeviceLinkError("This device-link request is incomplete.", 400);
  }

  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let totalBytes = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    totalBytes += value.byteLength;
    if (totalBytes > MAX_DEVICE_LINK_REQUEST_BYTES) {
      await reader.cancel().catch(() => undefined);
      throw new DeviceLinkError("Device-link request is too large.", 413);
    }
    chunks.push(value);
  }

  const encoded = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    encoded.set(chunk, offset);
    offset += chunk.byteLength;
  }

  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(encoded));
  } catch {
    throw new DeviceLinkError("This device-link request is invalid.", 400);
  }
  return parseDeviceLinkRequest(value);
}

export async function approveCurrentAccountDeviceLink(
  input: HostedDeviceLinkRequest
): Promise<HostedDeviceLinkResponse> {
  return withCurrentAccount((config, account) =>
    hostedDeviceApproveLink(config, account, input)
  );
}

export async function currentAccountDeviceLinkStatus(
  input: HostedDeviceLinkRequest
): Promise<HostedDeviceLinkResponse> {
  return withCurrentAccount((config, account) =>
    hostedDeviceLinkStatus(config, account, input)
  );
}

export async function currentHostedWebAccountBinding(): Promise<HostedWebAccountBinding> {
  return withCurrentAccount(async (config, account) =>
    projectHostedWebAccountBinding(await hostedDeviceState(config, account))
  );
}

export function projectHostedWebAccountBinding(value: unknown): HostedWebAccountBinding {
  if (!value || typeof value !== "object") {
    throw new Error("Hosted Web Device returned an invalid identity.");
  }
  const identity = (value as Record<string, unknown>).identity;
  if (!identity || typeof identity !== "object" || Array.isArray(identity)) {
    throw new Error("Hosted Web Device returned an invalid identity.");
  }
  const accountId = (identity as Record<string, unknown>).account_id;
  if (typeof accountId !== "string" || !NOSTR_ACCOUNT_ID_PATTERN.test(accountId)) {
    throw new Error("Hosted Web Device returned an invalid identity.");
  }

  // Project an exact allowlist. Device keys, signer material, and the Hosted
  // Web Device identifier must never cross this browser-facing boundary.
  return { account_id: accountId };
}

export function deviceLinkBoundaryError(error: unknown): DeviceLinkError {
  if (error instanceof DeviceLinkError) {
    return error;
  }
  if (error instanceof HostedDeviceRequestError) {
    switch (error.status) {
      case 400:
        return new DeviceLinkError("This device-link request is invalid.", 400);
      case 404:
        return new DeviceLinkError("This device-link request was not found.", 404);
      case 409:
        return new DeviceLinkError(
          "This Device cannot be linked from its current state.",
          409
        );
      case 410:
        return new DeviceLinkError("This device-link request expired.", 410);
    }
  }
  return new DeviceLinkError("Device linking is unavailable right now.", 502);
}

export function deviceLinkRouteError(error: unknown): DeviceLinkError {
  return error instanceof DeviceLinkError
    ? error
    : new DeviceLinkError("Device linking is unavailable right now.", 500);
}

async function withCurrentAccount<T>(
  operation: (
    config: NonNullable<ReturnType<typeof hostedDeviceConfig>>,
    account: Awaited<ReturnType<typeof getAccountAuthContext>>
  ) => Promise<T>
): Promise<T> {
  const account = await getAccountAuthContext();
  if (
    !account.workosUserId ||
    !account.emailVerified ||
    (account.source !== "workos" && account.source !== "dev")
  ) {
    throw new DeviceLinkError("Sign in again to use this Device.", 401);
  }

  let config: ReturnType<typeof hostedDeviceConfig>;
  try {
    config = hostedDeviceConfig();
  } catch {
    throw new DeviceLinkError("Device linking is not configured.", 503);
  }
  if (!config) {
    throw new DeviceLinkError("Device linking is not configured.", 503);
  }

  try {
    return await operation(config, account);
  } catch (error) {
    throw deviceLinkBoundaryError(error);
  }
}

function deviceLinkToken(field: string, value: unknown) {
  if (
    typeof value !== "string" ||
    !value ||
    new TextEncoder().encode(value).byteLength > MAX_DEVICE_LINK_TOKEN_BYTES ||
    value.trim() !== value ||
    /[\p{Cc}\p{Cf}]/u.test(value)
  ) {
    throw new DeviceLinkError(`The ${field} is invalid.`, 400);
  }
  return value;
}
