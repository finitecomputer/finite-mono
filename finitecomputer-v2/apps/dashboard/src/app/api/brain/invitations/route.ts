import { getAccountAuthContext } from "@/lib/dashboard-auth";
import {
  BrainHostedClientError,
  brainServerOrigin,
  hostedSignedBrainRequest,
} from "@/lib/brain-hosted-client";
import { hostedDeviceConfig } from "@/lib/hosted-web-device";
import { requestOriginSameOrNone } from "@/lib/http-headers";

const NO_STORE = { "cache-control": "no-store" };

/// The account's pending Brain invitations (the invitation card's data).
export async function GET(request: Request) {
  if (!requestOriginSameOrNone(request)) {
    return Response.json({ error: "Invitations require the dashboard." }, { status: 403, headers: NO_STORE });
  }
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    return Response.json({ error: "Sign in again to see invitations." }, { status: 401, headers: NO_STORE });
  }
  const config = hostedDeviceConfig();
  const brainOrigin = brainServerOrigin();
  if (!config || !brainOrigin) {
    return Response.json({ error: "Brain isn't available right now." }, { status: 503, headers: NO_STORE });
  }
  try {
    const result = await hostedSignedBrainRequest(
      config,
      account,
      brainOrigin,
      "GET",
      "/v1/my-invitations"
    );
    return Response.json(result ?? { invitations: [] }, { headers: NO_STORE });
  } catch (error) {
    if (error instanceof BrainHostedClientError) {
      return Response.json({ error: error.message }, { status: 502, headers: NO_STORE });
    }
    return Response.json({ error: "Invitations are unavailable right now." }, { status: 502, headers: NO_STORE });
  }
}
