import { getAccountAuthContext } from "@/lib/dashboard-auth";
import {
  HostedDeviceRequestError,
  hostedDeviceApproveLink,
  hostedDeviceConfig,
  hostedDeviceLinkStatus,
  type HostedDeviceLinkRequest,
  type HostedDeviceLinkResponse,
} from "@/lib/hosted-web-device";

const MAX_DEVICE_LINK_TOKEN_BYTES = 256;
export const MAX_DEVICE_LINK_REQUEST_BYTES = 4 * 1024;

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
  if (!value || typeof value !== "object") {
    throw new DeviceLinkError("This device-link request is incomplete.", 400);
  }
  const record = value as Record<string, unknown>;
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
  const contentType = request.headers.get("content-type")?.toLowerCase() ?? "";
  if (!contentType.startsWith("application/json")) {
    throw new DeviceLinkError("Device-link requests must use JSON.", 415);
  }
  const declaredLength = Number(request.headers.get("content-length") ?? "0");
  if (
    Number.isFinite(declaredLength) &&
    declaredLength > MAX_DEVICE_LINK_REQUEST_BYTES
  ) {
    throw new DeviceLinkError("Device-link request is too large.", 413);
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

async function withCurrentAccount(
  operation: (
    config: NonNullable<ReturnType<typeof hostedDeviceConfig>>,
    account: Awaited<ReturnType<typeof getAccountAuthContext>>
  ) => Promise<HostedDeviceLinkResponse>
) {
  const account = await getAccountAuthContext();
  if (
    !account.workosUserId ||
    !account.emailVerified ||
    (account.source !== "workos" && account.source !== "dev")
  ) {
    throw new DeviceLinkError("Sign in again to approve this Device.", 401);
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

export function deviceLinkBoundaryError(error: unknown): DeviceLinkError {
  if (error instanceof HostedDeviceRequestError) {
    const status = [400, 404, 409, 410].includes(error.status)
      ? error.status
      : 502;
    return new DeviceLinkError(
      status === 502
        ? "Device linking is unavailable right now."
        : error.message,
      status
    );
  }
  if (error instanceof DeviceLinkError) {
    return error;
  }
  return new DeviceLinkError("Device linking is unavailable right now.", 502);
}

function deviceLinkToken(field: string, value: unknown) {
  if (
    typeof value !== "string" ||
    !value ||
    value.length > MAX_DEVICE_LINK_TOKEN_BYTES ||
    value.trim() !== value ||
    Array.from(value).some((character) => /\p{Cc}/u.test(character))
  ) {
    throw new DeviceLinkError(`The ${field} is invalid.`, 400);
  }
  return value;
}
