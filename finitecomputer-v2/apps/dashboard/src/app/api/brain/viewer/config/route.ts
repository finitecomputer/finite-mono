import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { brainPublicOrigin, brainServerOrigin } from "@/lib/brain-hosted-client";
import { requestOriginSameOrNone } from "@/lib/http-headers";

const NO_STORE = { "cache-control": "no-store" };

/// The origin the browser viewer signs its NIP-98 events against and calls
/// directly (records + SSE). This is the public origin the Brain server
/// validates in the `u` tag, which may differ from the upstream origin
/// behind a proxy.
export async function GET(request: Request) {
  if (!requestOriginSameOrNone(request)) {
    return Response.json({ error: "Viewer config requires the dashboard." }, { status: 403, headers: NO_STORE });
  }
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    return Response.json({ error: "Sign in again to view Brain documents." }, { status: 401, headers: NO_STORE });
  }
  const origin = brainPublicOrigin() ?? brainServerOrigin();
  if (!origin) {
    return Response.json({ error: "Brain isn't available right now." }, { status: 503, headers: NO_STORE });
  }
  return Response.json({ origin }, { headers: NO_STORE });
}
