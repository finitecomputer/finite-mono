import {
  brainCapabilityMatchesCurrentAccount,
  brainIdentityRequestHash,
  parseBrainIdentityProviderRequest,
  verifyBrainClientCapability,
  verifyBrainSessionProof,
} from "@/lib/brain-identity-provider";
import {
  HostedDeviceRequestError,
  hostedDeviceBrainIdentityProvider,
  hostedDeviceConfig,
} from "@/lib/hosted-web-device";

const MAX_PROVIDER_REQUEST_BYTES = 1024 * 1024;
const BRAIN_CLIENT_CAPABILITY_HEADER = "x-finite-brain-client-capability";
const BRAIN_SESSION_PROOF_HEADER = "x-finite-brain-session-proof";
const PROVIDER_HEADERS = {
  "access-control-allow-origin": "null",
  "cache-control": "no-store",
  vary: "origin",
};

export function OPTIONS(request: Request) {
  if (request.headers.get("origin") !== "null") {
    return new Response(null, { status: 403, headers: PROVIDER_HEADERS });
  }
  return new Response(null, {
    status: 204,
    headers: {
      ...PROVIDER_HEADERS,
      "access-control-allow-headers":
        "content-type, x-finite-brain-client-capability, x-finite-brain-provider-version, x-finite-brain-session-proof",
      "access-control-allow-methods": "POST",
      "access-control-max-age": "600",
    },
  });
}

export async function POST(request: Request) {
  if (request.headers.get("origin") !== "null") {
    return Response.json(
      { error: "The Brain identity adapter requires the isolated Brain client." },
      { status: 403, headers: PROVIDER_HEADERS }
    );
  }
  const config = hostedDeviceConfig();
  if (!config) {
    return Response.json(
      { error: "Brain identity is not available right now." },
      { status: 503, headers: PROVIDER_HEADERS }
    );
  }
  const capability = verifyBrainClientCapability(
    request.headers.get(BRAIN_CLIENT_CAPABILITY_HEADER),
    config.apiToken,
  );
  if (!capability) {
    return Response.json(
      { error: "Open Brain from the dashboard again." },
      { status: 401, headers: PROVIDER_HEADERS },
    );
  }
  try {
    const text = await request.text();
    if (new TextEncoder().encode(text).byteLength > MAX_PROVIDER_REQUEST_BYTES) {
      return Response.json(
        { error: "Brain identity request is too large." },
        { status: 413, headers: PROVIDER_HEADERS }
      );
    }
    const session = verifyBrainSessionProof(
      request.headers.get(BRAIN_SESSION_PROOF_HEADER),
      config.apiToken,
      brainIdentityRequestHash(text),
    );
    if (!session || !brainCapabilityMatchesCurrentAccount(capability, session)) {
      return Response.json(
        { error: "Your dashboard session expired. Sign in and open Brain again." },
        { status: 401, headers: PROVIDER_HEADERS },
      );
    }
    const account = {
      email: null,
      workosUserId: session.workosUserId,
      emailVerified: true,
      source: "workos" as const,
    };
    const input = parseBrainIdentityProviderRequest(JSON.parse(text));
    const result = await hostedDeviceBrainIdentityProvider(
      config,
      account,
      input,
      capability.brainPublicOrigin,
    );
    return Response.json(result, { headers: PROVIDER_HEADERS });
  } catch (error) {
    if (error instanceof HostedDeviceRequestError) {
      return Response.json(
        { error: error.message },
        { status: error.status, headers: PROVIDER_HEADERS }
      );
    }
    return Response.json(
      { error: "Brain identity request is invalid." },
      { status: 400, headers: PROVIDER_HEADERS }
    );
  }
}
