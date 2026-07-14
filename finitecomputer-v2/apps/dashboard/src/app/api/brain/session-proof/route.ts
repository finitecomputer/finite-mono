import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { issueBrainSessionProof } from "@/lib/brain-identity-provider";
import { hostedDeviceConfig } from "@/lib/hosted-web-device";
import { browserVisibleRequestOrigin } from "@/lib/http-headers";

const MAX_SESSION_PROOF_REQUEST_BYTES = 1024;
const NO_STORE_HEADERS = { "cache-control": "no-store" };

export async function POST(request: Request) {
  const dashboardOrigin = browserVisibleRequestOrigin(request);
  if (!dashboardOrigin || request.headers.get("origin") !== dashboardOrigin) {
    return Response.json(
      { error: "Brain session proof requires the signed-in dashboard." },
      { status: 403, headers: NO_STORE_HEADERS },
    );
  }
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    return Response.json(
      { error: "Sign in and open Brain again." },
      { status: 401, headers: NO_STORE_HEADERS },
    );
  }
  const config = hostedDeviceConfig();
  if (!config) {
    return Response.json(
      { error: "Brain identity is not available right now." },
      { status: 503, headers: NO_STORE_HEADERS },
    );
  }
  const text = await request.text();
  if (new TextEncoder().encode(text).byteLength > MAX_SESSION_PROOF_REQUEST_BYTES) {
    return Response.json(
      { error: "Brain session proof request is too large." },
      { status: 413, headers: NO_STORE_HEADERS },
    );
  }
  try {
    const value = JSON.parse(text) as Record<string, unknown>;
    if (
      !value ||
      typeof value !== "object" ||
      Object.keys(value).length !== 1 ||
      typeof value.requestHash !== "string" ||
      !/^[0-9a-f]{64}$/u.test(value.requestHash)
    ) {
      throw new Error("invalid Brain session proof request");
    }
    return Response.json(
      {
        proof: issueBrainSessionProof(
          config.apiToken,
          account.workosUserId,
          value.requestHash,
        ),
      },
      { headers: NO_STORE_HEADERS },
    );
  } catch {
    return Response.json(
      { error: "Brain session proof request is invalid." },
      { status: 400, headers: NO_STORE_HEADERS },
    );
  }
}
