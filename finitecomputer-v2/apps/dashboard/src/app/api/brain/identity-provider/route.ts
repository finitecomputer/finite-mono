import { getAccountAuthContext } from "@/lib/dashboard-auth";
import {
  officialBrainClientRequest,
  parseBrainIdentityProviderRequest,
} from "@/lib/brain-identity-provider";
import {
  HostedDeviceRequestError,
  hostedDeviceBrainIdentityProvider,
  hostedDeviceConfig,
} from "@/lib/hosted-web-device";

const MAX_PROVIDER_REQUEST_BYTES = 1024 * 1024;
const NO_STORE_HEADERS = { "cache-control": "no-store" };

export async function POST(request: Request) {
  if (!officialBrainClientRequest(request.url, request.headers.get("referer"))) {
    return Response.json(
      { error: "The Brain identity adapter is available only to the official Brain client." },
      { status: 403, headers: NO_STORE_HEADERS }
    );
  }
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    return Response.json(
      { error: "Sign in again to open Brain." },
      { status: 401, headers: NO_STORE_HEADERS }
    );
  }
  const config = hostedDeviceConfig();
  if (!config) {
    return Response.json(
      { error: "Brain identity is not available right now." },
      { status: 503, headers: NO_STORE_HEADERS }
    );
  }

  try {
    const text = await request.text();
    if (new TextEncoder().encode(text).byteLength > MAX_PROVIDER_REQUEST_BYTES) {
      return Response.json(
        { error: "Brain identity request is too large." },
        { status: 413, headers: NO_STORE_HEADERS }
      );
    }
    const input = parseBrainIdentityProviderRequest(JSON.parse(text));
    const result = await hostedDeviceBrainIdentityProvider(
      config,
      account,
      input,
      new URL(request.url).origin
    );
    return Response.json(result, { headers: NO_STORE_HEADERS });
  } catch (error) {
    if (error instanceof HostedDeviceRequestError) {
      return Response.json(
        { error: error.message },
        { status: error.status, headers: NO_STORE_HEADERS }
      );
    }
    return Response.json(
      { error: "Brain identity request is invalid." },
      { status: 400, headers: NO_STORE_HEADERS }
    );
  }
}
