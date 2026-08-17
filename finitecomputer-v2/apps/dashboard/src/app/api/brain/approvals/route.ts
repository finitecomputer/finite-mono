import { getAccountAuthContext } from "@/lib/dashboard-auth";
import {
  BrainHostedClientError,
  brainServerOrigin,
  hostedSignedBrainRequest,
} from "@/lib/brain-hosted-client";
import { hostedDeviceConfig } from "@/lib/hosted-web-device";
import { requestOriginMatchesHost } from "@/lib/http-headers";

const NO_STORE = { "cache-control": "no-store" };

type ApprovalCard = {
  id: string;
  brainId: string;
  brainName: string;
  action: string;
  requestedByNpub: string;
  expiresAt: number;
  createdAt: string;
  payload: unknown;
};

/// Pending Brain approval requests across the account's admin brains,
/// signed by the human principal through the hosted chat device.
export async function GET(request: Request) {
  if (!requestOriginMatchesHost(request)) {
    return Response.json({ error: "Brain approvals require the dashboard." }, { status: 403, headers: NO_STORE });
  }
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    return Response.json({ error: "Sign in again to see Brain approvals." }, { status: 401, headers: NO_STORE });
  }
  const config = hostedDeviceConfig();
  if (!config) {
    return Response.json({ error: "Brain identity is not available right now." }, { status: 503, headers: NO_STORE });
  }
  const brainOrigin = brainServerOrigin();
  if (!brainOrigin) {
    return Response.json({ error: "Brain isn't available right now." }, { status: 503, headers: NO_STORE });
  }
  try {
    const brains = (await hostedSignedBrainRequest(
      config,
      account,
      brainOrigin,
      "GET",
      "/v1/brains"
    )) as { brains?: Array<{ brainId: string; name?: string; role?: string }> };
    const cards: ApprovalCard[] = [];
    for (const brain of brains?.brains ?? []) {
      if (brain.role && !["admin", "owner", "personal"].includes(brain.role)) continue;
      const listed = (await hostedSignedBrainRequest(
        config,
        account,
        brainOrigin,
        "GET",
        `/v1/brains/${encodeURIComponent(brain.brainId)}/approval-requests`
      )) as { requests?: Array<Record<string, unknown>> };
      for (const request of listed?.requests ?? []) {
        if (request.status !== "pending") continue;
        cards.push({
          id: String(request.id ?? ""),
          brainId: brain.brainId,
          brainName: brain.name ?? brain.brainId,
          action: String(request.action ?? ""),
          requestedByNpub: String(request.requestedByNpub ?? ""),
          expiresAt: Number(request.expiresAt ?? 0),
          createdAt: String(request.createdAt ?? ""),
          payload: request.payload ?? null,
        });
      }
    }
    return Response.json({ approvals: cards }, { headers: NO_STORE });
  } catch (error) {
    if (error instanceof BrainHostedClientError) {
      return Response.json({ error: error.message }, { status: 502, headers: NO_STORE });
    }
    return Response.json({ error: "Brain approvals are unavailable right now." }, { status: 502, headers: NO_STORE });
  }
}
