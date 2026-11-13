import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { brainServerOrigin } from "@/lib/brain-hosted-client";
import {
  brainDeletionFailure,
  deleteBrainExactly,
  hostedBrainDeletionTransport,
  previewBrainDeletion,
} from "@/lib/brain-deletion";
import { BRAIN_ID_PATTERN, STATE_DIGEST_PATTERN } from "@/lib/brain-deletion-view";
import { hostedDeviceConfig } from "@/lib/hosted-web-device";
import { requestOriginMatchesHost, requestOriginSameOrNone } from "@/lib/http-headers";

const NO_STORE = { "cache-control": "no-store" };
const MAX_BODY_BYTES = 4 * 1024;

async function deletionTransport() {
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    return { error: Response.json({ error: "Sign in again to manage this Brain." }, { status: 401, headers: NO_STORE }) };
  }
  const config = hostedDeviceConfig();
  const brainOrigin = brainServerOrigin();
  if (!config || !brainOrigin) {
    return { error: Response.json({ error: "Brain isn't available right now." }, { status: 503, headers: NO_STORE }) };
  }
  return { transport: hostedBrainDeletionTransport(config, account, brainOrigin) };
}

/// Preview deleting an Organization Brain with the account's hosted Brain key.
export async function GET(request: Request) {
  if (!requestOriginSameOrNone(request)) {
    return Response.json({ error: "Brain deletion requires the dashboard." }, { status: 403, headers: NO_STORE });
  }
  const brainId = new URL(request.url).searchParams.get("brainId")?.trim() ?? "";
  if (!BRAIN_ID_PATTERN.test(brainId)) {
    return Response.json({ error: "Brain deletion request is invalid." }, { status: 400, headers: NO_STORE });
  }
  const { transport, error } = await deletionTransport();
  if (!transport) return error;
  try {
    return Response.json(await previewBrainDeletion(transport, brainId), { headers: NO_STORE });
  } catch (caught) {
    const failure = brainDeletionFailure(caught);
    return Response.json({ error: failure.message }, { status: failure.status, headers: NO_STORE });
  }
}

/// Delete exactly the previewed state. The account confirms by typing the
/// Brain id; the Brain server rechecks admin standing and the state digest.
export async function POST(request: Request) {
  if (!requestOriginMatchesHost(request)) {
    return Response.json({ error: "Brain deletion requires the dashboard." }, { status: 403, headers: NO_STORE });
  }
  const bodyText = await request.text();
  if (new TextEncoder().encode(bodyText).byteLength > MAX_BODY_BYTES) {
    return Response.json({ error: "Brain deletion request is too large." }, { status: 413, headers: NO_STORE });
  }
  let body: { brainId?: unknown; stateDigest?: unknown; confirmation?: unknown };
  try {
    body = JSON.parse(bodyText);
  } catch {
    return Response.json({ error: "Brain deletion request is invalid." }, { status: 400, headers: NO_STORE });
  }
  const brainId = typeof body.brainId === "string" ? body.brainId : "";
  const stateDigest = typeof body.stateDigest === "string" ? body.stateDigest : "";
  if (!BRAIN_ID_PATTERN.test(brainId) || !STATE_DIGEST_PATTERN.test(stateDigest)) {
    return Response.json({ error: "Brain deletion request is invalid." }, { status: 400, headers: NO_STORE });
  }
  if (body.confirmation !== brainId) {
    return Response.json(
      { error: "Type the Brain ID exactly to confirm deletion." },
      { status: 400, headers: NO_STORE }
    );
  }
  const { transport, error } = await deletionTransport();
  if (!transport) return error;
  try {
    return Response.json(await deleteBrainExactly(transport, { brainId, stateDigest }), { headers: NO_STORE });
  } catch (caught) {
    const failure = brainDeletionFailure(caught);
    return Response.json({ error: failure.message }, { status: failure.status, headers: NO_STORE });
  }
}
