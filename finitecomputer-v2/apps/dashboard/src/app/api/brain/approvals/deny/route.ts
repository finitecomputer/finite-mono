import { getAccountAuthContext } from "@/lib/dashboard-auth";
import {
  BrainHostedClientError,
  brainServerOrigin,
  hostedSignedBrainRequest,
} from "@/lib/brain-hosted-client";
import { hostedDeviceConfig } from "@/lib/hosted-web-device";
import { requestOriginMatchesHost } from "@/lib/http-headers";

const NO_STORE = { "cache-control": "no-store" };
const MAX_BODY_BYTES = 8 * 1024;

/// Dismiss one pending Brain approval request without signing anything.
export async function POST(request: Request) {
  if (!requestOriginMatchesHost(request)) {
    return Response.json({ error: "Approvals require the dashboard." }, { status: 403, headers: NO_STORE });
  }
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    return Response.json({ error: "Sign in again." }, { status: 401, headers: NO_STORE });
  }
  const config = hostedDeviceConfig();
  const brainOrigin = brainServerOrigin();
  if (!config || !brainOrigin) {
    return Response.json({ error: "Brain isn't available right now." }, { status: 503, headers: NO_STORE });
  }
  const text = await request.text();
  if (new TextEncoder().encode(text).byteLength > MAX_BODY_BYTES) {
    return Response.json({ error: "Deny request is too large." }, { status: 413, headers: NO_STORE });
  }
  let body: { brainId?: string; requestId?: string };
  try {
    body = JSON.parse(text);
  } catch {
    return Response.json({ error: "Deny request is invalid." }, { status: 400, headers: NO_STORE });
  }
  const brainId = body.brainId?.trim();
  const requestId = body.requestId?.trim();
  if (!brainId || !requestId || !/^[a-z0-9][a-z0-9_-]{0,127}$/u.test(brainId)) {
    return Response.json({ error: "Deny request is invalid." }, { status: 400, headers: NO_STORE });
  }
  try {
    const result = await hostedSignedBrainRequest(
      config,
      account,
      brainOrigin,
      "POST",
      `/v1/brains/${encodeURIComponent(brainId)}/approval-requests/${encodeURIComponent(requestId)}/deny`
    );
    return Response.json(result ?? { status: "ok" }, { headers: NO_STORE });
  } catch (error) {
    if (error instanceof BrainHostedClientError) {
      return Response.json({ error: error.message }, { status: 502, headers: NO_STORE });
    }
    return Response.json({ error: "The denial could not be recorded right now." }, { status: 502, headers: NO_STORE });
  }
}
