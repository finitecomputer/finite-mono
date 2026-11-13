import { getAccountAuthContext } from "@/lib/dashboard-auth";
import {
  BrainHostedClientError,
  brainPublicOrigin,
  brainServerOrigin,
  hostedSignedBrainRequest,
} from "@/lib/brain-hosted-client";
import {
  HostedDeviceRequestError,
  hostedDeviceBrainIdentityProvider,
  hostedDeviceConfig,
} from "@/lib/hosted-web-device";
import { requestOriginMatchesHost, requestOriginSameOrNone } from "@/lib/http-headers";
import {
  MAX_SETUP_BODY_BYTES,
  brainRefusal,
  createdPersonalBrainId,
  isNpub,
  parsePersonalBrainSetupRequest,
  personalBrainConflictMessage,
  personalBrainCreateBody,
} from "@/lib/personal-brain-setup";

const NO_STORE = { "cache-control": "no-store" };
const SETUP_FAILED = "Your Personal Brain could not be set up right now.";

function failure(error: string, status: number) {
  return Response.json({ error }, { status, headers: NO_STORE });
}

async function context(request: Request, sameOrigin: (request: Request) => boolean) {
  if (!sameOrigin(request)) return { response: failure("Setup requires the dashboard.", 403) };
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    return { response: failure("Sign in again to set up your Personal Brain.", 401) };
  }
  const config = hostedDeviceConfig();
  const brainOrigin = brainServerOrigin();
  if (!config || !brainOrigin) return { response: failure("Brain isn't available right now.", 503) };
  return { account, config, brainOrigin };
}

function setupFailure(error: unknown) {
  // The Hosted Device has no Brain key for this account until chat sets it
  // up. Nothing here creates one.
  if (error instanceof HostedDeviceRequestError && error.status === 428) {
    return failure("Open chat once to finish setting up your account, then try again.", 409);
  }
  if (error instanceof BrainHostedClientError) {
    if (error.status === 409) return failure(personalBrainConflictMessage(error.message), 409);
    console.warn(`Personal Brain setup refused by Brain: HTTP ${error.status}`);
    return failure(brainRefusal(error.status), 502);
  }
  return failure(SETUP_FAILED, 502);
}

/// The person's existing hosted Brain key, so the selected Agent can consent
/// to this exact owner. Identify only: this never mints a key.
export async function GET(request: Request) {
  const setup = await context(request, requestOriginSameOrNone);
  if ("response" in setup) return setup.response;
  try {
    const identity = await hostedDeviceBrainIdentityProvider(
      setup.config,
      setup.account,
      { version: "finite-brain-identity-provider-v1", operation: "identifyMember", input: {} },
      brainPublicOrigin() ?? setup.brainOrigin
    );
    if (!isNpub(identity.npub)) return failure(SETUP_FAILED, 502);
    return Response.json({ ownerNpub: identity.npub }, { headers: NO_STORE });
  } catch (error) {
    return setupFailure(error);
  }
}

/// Create the Personal Brain with the person's signature over a body that
/// carries the Agent's signed consent. The Brain server verifies both.
export async function POST(request: Request) {
  const setup = await context(request, requestOriginMatchesHost);
  if ("response" in setup) return setup.response;
  const text = await request.text();
  if (new TextEncoder().encode(text).byteLength > MAX_SETUP_BODY_BYTES) {
    return failure("Setup request is too large.", 413);
  }
  let body: unknown;
  try {
    body = JSON.parse(text);
  } catch {
    return failure("Setup request is invalid.", 400);
  }
  const personal = parsePersonalBrainSetupRequest(body);
  if (!personal) return failure("Setup request is invalid.", 400);
  try {
    const result = await hostedSignedBrainRequest(
      setup.config,
      setup.account,
      setup.brainOrigin,
      "POST",
      "/v1/brains",
      personalBrainCreateBody(personal)
    );
    return Response.json({ brainId: createdPersonalBrainId(result, personal.brainId) }, { headers: NO_STORE });
  } catch (error) {
    return setupFailure(error);
  }
}
