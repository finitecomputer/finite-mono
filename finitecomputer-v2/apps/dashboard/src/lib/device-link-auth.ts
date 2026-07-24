import { WorkOS } from "@workos-inc/node";
import {
  createRemoteJWKSet,
  jwtVerify,
  type JWTPayload,
} from "jose";

import {
  getAccountAuthContext,
  type AccountAuthContext,
} from "@/lib/dashboard-auth";
import { DeviceLinkError } from "@/lib/device-link";

const MAX_BEARER_TOKEN_BYTES = 16 * 1024;
const WORKOS_ISSUERS = [
  "https://api.workos.com",
  "https://api.workos.com/",
] as const;
const remoteKeySets = new Map<string, ReturnType<typeof createRemoteJWKSet>>();

type VerifiedBearer = {
  subject: string;
  organizationId: string | null;
};

type DeviceLinkBearerDependencies = {
  verify: (token: string, clientId: string) => Promise<VerifiedBearer>;
  getUser: (
    userId: string,
    apiKey: string
  ) => Promise<{ id: string; email: string; emailVerified: boolean }>;
};

const defaultDependencies: DeviceLinkBearerDependencies = {
  verify: verifyDeviceLinkBearerToken,
  async getUser(userId, apiKey) {
    const user = await new WorkOS(apiKey).userManagement.getUser(userId);
    return {
      id: user.id,
      email: user.email,
      emailVerified: user.emailVerified,
    };
  },
};

/**
 * Authenticate only the two native device-link routes with a WorkOS bearer
 * token. Browser and Electron callers continue through the existing encrypted
 * AuthKit cookie path.
 */
export async function getDeviceLinkAccountAuthContext(
  request: Request,
  env: Record<string, string | undefined> = process.env,
  dependencies: DeviceLinkBearerDependencies = defaultDependencies
): Promise<AccountAuthContext> {
  const authorization = request.headers.get("authorization");
  if (authorization === null) {
    return getAccountAuthContext();
  }
  const token = parseBearerToken(authorization);
  const clientId = requiredWorkosValue("FC_WORKOS_IOS_CLIENT_ID", env);
  const apiKey = requiredWorkosValue("WORKOS_API_KEY", env);

  try {
    const verified = await dependencies.verify(token, clientId);
    const user = await dependencies.getUser(verified.subject, apiKey);
    if (user.id !== verified.subject) {
      throw new Error("WorkOS user mismatch");
    }
    return {
      email: user.email,
      workosUserId: user.id,
      emailVerified: user.emailVerified,
      organizationId: verified.organizationId,
      source: "workos",
    };
  } catch {
    throw new DeviceLinkError("Sign in again to use this Device.", 401);
  }
}

export async function verifyDeviceLinkBearerToken(
  token: string,
  clientId: string,
  key: Parameters<typeof jwtVerify>[1] = remoteKeySet(clientId)
): Promise<VerifiedBearer> {
  const { payload } = await jwtVerify(token, key, {
    algorithms: ["RS256"],
    issuer: [...WORKOS_ISSUERS],
    requiredClaims: ["iss", "sub", "sid", "jti", "exp", "iat"],
  });
  return verifiedBearerClaims(payload, clientId);
}

export function verifiedBearerClaims(
  payload: JWTPayload,
  clientId: string
): VerifiedBearer {
  if (
    payload.client_id !== clientId ||
    typeof payload.sub !== "string" ||
    !payload.sub.startsWith("user_") ||
    typeof payload.sid !== "string" ||
    !payload.sid.startsWith("session_") ||
    typeof payload.jti !== "string" ||
    !payload.jti
  ) {
    throw new Error("Invalid WorkOS access token");
  }
  return {
    subject: payload.sub,
    organizationId:
      typeof payload.org_id === "string" && payload.org_id.trim()
        ? payload.org_id.trim()
        : null,
  };
}

export function parseBearerToken(value: string) {
  if (
    value.length > MAX_BEARER_TOKEN_BYTES ||
    /[\p{Cc}\p{Cf}]/u.test(value)
  ) {
    throw new DeviceLinkError("Sign in again to use this Device.", 401);
  }
  const match = /^Bearer ([A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+)$/u.exec(
    value
  );
  if (!match) {
    throw new DeviceLinkError("Sign in again to use this Device.", 401);
  }
  return match[1];
}

function remoteKeySet(clientId: string) {
  let keys = remoteKeySets.get(clientId);
  if (!keys) {
    keys = createRemoteJWKSet(
      new URL(`https://api.workos.com/sso/jwks/${encodeURIComponent(clientId)}`),
      {
        timeoutDuration: 5_000,
        cooldownDuration: 30_000,
      }
    );
    remoteKeySets.set(clientId, keys);
  }
  return keys;
}

function requiredWorkosValue(
  name: "WORKOS_API_KEY" | "FC_WORKOS_IOS_CLIENT_ID",
  env: Record<string, string | undefined>
) {
  const value = env[name]?.trim();
  if (!value) {
    throw new DeviceLinkError("Device linking is not configured.", 503);
  }
  return value;
}
