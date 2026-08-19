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
const ID_PATTERN = /^[a-z0-9][a-z0-9_-]{0,127}$/u;
const DEFAULT_TTL_SECS = 3_600;
const MAX_TTL_SECS = 86_400;

/// Mint a viewer session for the browser's ephemeral key: the requesting
/// principal is the signed-in account speaking through the hosted web
/// device (the brain:// viewer's owner-proof step). The Brain server
/// re-checks Folder access and stores only a pending key-delivery record.
export async function POST(request: Request) {
  if (!requestOriginMatchesHost(request)) {
    return Response.json({ error: "Viewer sessions require the dashboard." }, { status: 403, headers: NO_STORE });
  }
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    return Response.json({ error: "Sign in again to view Brain documents." }, { status: 401, headers: NO_STORE });
  }
  const config = hostedDeviceConfig();
  const brainOrigin = brainServerOrigin();
  if (!config || !brainOrigin) {
    return Response.json({ error: "Brain isn't available right now." }, { status: 503, headers: NO_STORE });
  }
  const text = await request.text();
  if (new TextEncoder().encode(text).byteLength > MAX_BODY_BYTES) {
    return Response.json({ error: "Viewer session request is too large." }, { status: 413, headers: NO_STORE });
  }
  let body: { brainId?: string; folderId?: string; ephemeralNpub?: string; requestedTtlSecs?: number };
  try {
    body = JSON.parse(text);
  } catch {
    return Response.json({ error: "Viewer session request is invalid." }, { status: 400, headers: NO_STORE });
  }
  const brainId = body.brainId?.trim();
  const folderId = body.folderId?.trim();
  const ephemeralNpub = body.ephemeralNpub?.trim();
  if (
    !brainId ||
    !folderId ||
    !ID_PATTERN.test(brainId) ||
    !ID_PATTERN.test(folderId) ||
    !ephemeralNpub?.startsWith("npub1")
  ) {
    return Response.json({ error: "Viewer session request is invalid." }, { status: 400, headers: NO_STORE });
  }
  const requestedTtlSecs =
    typeof body.requestedTtlSecs === "number" && Number.isFinite(body.requestedTtlSecs)
      ? Math.min(Math.max(Math.floor(body.requestedTtlSecs), 1), MAX_TTL_SECS)
      : DEFAULT_TTL_SECS;
  try {
    const session = await hostedSignedBrainRequest(
      config,
      account,
      brainOrigin,
      "POST",
      "/v1/viewer-session-requests",
      JSON.stringify({ brainId, folderId, ephemeralNpub, requestedTtlSecs })
    );
    return Response.json(session ?? {}, { headers: NO_STORE });
  } catch (error) {
    if (error instanceof BrainHostedClientError) {
      return Response.json({ error: error.message }, { status: 502, headers: NO_STORE });
    }
    return Response.json({ error: "Viewer sessions are unavailable right now." }, { status: 502, headers: NO_STORE });
  }
}
