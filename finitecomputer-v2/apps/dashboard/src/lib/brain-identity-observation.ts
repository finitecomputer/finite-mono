import { randomBytes } from "node:crypto";

import { type AccountAuthContext } from "@/lib/dashboard-auth";
import {
  type HostedDeviceConfig,
  hostedDeviceBrainIdentityProvider,
} from "@/lib/hosted-web-device";

/// After a successful hosted human Brain action, tell Core (trusted service
/// observation, FIN-122) which existing hosted key this account used and that
/// the account shares its contact with that exact Brain's administrators.
///
/// Best effort and bounded: it never changes the action's result, never
/// mints a key (identifyMember only loads existing Device state) and never
/// trusts browser input for the account, key or Brain.

export const OBSERVATION_VERSION = "finite-core-brain-account-observation-v1";
export const OBSERVATION_PATH = "/api/core/internal/v1/brain-account-observations";
export const OBSERVATION_CREDENTIAL_HEADER = "x-finite-brain-observation-credential";
const ATTEMPT_TIMEOUT_MS = 2_000;
const MAX_ATTEMPTS = 2;

export type BrainObservationConfig = {
  coreUrl: string;
  credential: string;
};

/// Both variables or neither; anything else leaves the hook off.
export function brainObservationConfig(
  env: Record<string, string | undefined> = process.env
): BrainObservationConfig | null {
  const coreUrl = env.FC_CORE_BRAIN_IDENTITY_URL?.trim();
  const credential = env.FC_CORE_BRAIN_OBSERVATION_TOKEN?.trim();
  if (!coreUrl || !credential) return null;
  try {
    const url = new URL(coreUrl);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    if (url.pathname !== "/" || url.search || url.hash) return null;
    return { coreUrl: url.origin, credential };
  } catch {
    return null;
  }
}

/// The current Approve button sends `shareAccountContact: true`
/// beside its disclosure text. A request without it (for example from a
/// tab loaded before that text existed) still performs the action but never
/// records sharing. This is intent only; it carries no identity.
export function requestsContactSharing(body: unknown): boolean {
  return (
    !!body &&
    typeof body === "object" &&
    (body as Record<string, unknown>).shareAccountContact === true
  );
}

export type ObservationResult =
  | { outcome: "recorded" | "unchanged" }
  | { skipped: string };

type Dependencies = {
  identifyMember: () => Promise<unknown>;
  fetch: typeof fetch;
  now: () => Date;
};

/// The exact Brain and accepting key, read only from the Brain server's
/// acceptance response. A response that does not prove acceptance by this
/// hosted key yields nothing.
export function acceptedBrainInvitation(
  result: unknown,
  hostedNpub: string
): { brainId: string } | null {
  if (!result || typeof result !== "object") return null;
  const record = result as Record<string, unknown>;
  const brainId = record.brainId;
  if (typeof brainId !== "string" || !/^[A-Za-z0-9_-]{1,128}$/u.test(brainId)) return null;
  if (record.status !== "accepted") return null;
  const accepter = [record.userId, record.claimedByNpub].some(
    (value) => typeof value === "string" && value === hostedNpub
  );
  return accepter ? { brainId } : null;
}

/// A delegation-grant approval the Brain server applied. The Brain checked
/// the hosted signer's standing in the exact Brain named by the signed
/// request path, so that path's Brain id is the confirmed scope.
export function appliedBrainApproval(result: unknown, signedBrainId: string): { brainId: string } | null {
  if (!result || typeof result !== "object") return null;
  const record = result as Record<string, unknown>;
  if (record.status !== "applied" || record.action !== "delegation-grant") return null;
  if (!/^[A-Za-z0-9_-]{1,128}$/u.test(signedBrainId)) return null;
  return { brainId: signedBrainId };
}

export function observeHostedBrainInvitationAcceptance(
  config: BrainObservationConfig | null,
  account: AccountAuthContext,
  brainServer: string,
  acceptance: unknown,
  dependencies: Dependencies
): Promise<ObservationResult> {
  return observeHostedHumanBrainAction(
    config,
    account,
    brainServer,
    (npub) => acceptedBrainInvitation(acceptance, npub),
    dependencies
  );
}

/// Shared path for qualifying hosted human actions. `qualify` receives the
/// hosted key's npub and returns the Brain the action proved, or null.
export async function observeHostedHumanBrainAction(
  config: BrainObservationConfig | null,
  account: AccountAuthContext,
  brainServer: string,
  qualify: (hostedNpub: string) => { brainId: string } | null,
  dependencies: Dependencies
): Promise<ObservationResult> {
  if (!config) return { skipped: "not configured" };
  if (!account.accessToken) return { skipped: "no account session" };
  let identity: { publicKeyHex?: unknown; npub?: unknown };
  try {
    identity = (await dependencies.identifyMember()) as typeof identity;
  } catch {
    // Missing or half-present Device state: nothing to observe, nothing created.
    return { skipped: "no hosted identity" };
  }
  const publicKeyHex = identity?.publicKeyHex;
  const npub = identity?.npub;
  if (
    typeof publicKeyHex !== "string" ||
    !/^[0-9a-f]{64}$/u.test(publicKeyHex) ||
    typeof npub !== "string"
  ) {
    return { skipped: "no hosted identity" };
  }
  const accepted = qualify(npub);
  if (!accepted) return { skipped: "action not proven" };

  const operationId = `obs_${randomBytes(16).toString("hex")}`;
  const body = JSON.stringify({
    version: OBSERVATION_VERSION,
    operationId,
    brainServer,
    brainId: accepted.brainId,
    observedAt: dependencies.now().toISOString(),
    actionKind: "humanHostedAction",
    observedHumanPublicKeyHex: publicKeyHex,
    participatingPublicKeyHex: publicKeyHex,
  });
  // A lost response is retried with the same operation id; Core returns
  // the stored outcome instead of applying it twice.
  for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt += 1) {
    let response: Response;
    try {
      response = await dependencies.fetch(`${config.coreUrl}${OBSERVATION_PATH}`, {
        method: "POST",
        cache: "no-store",
        headers: {
          authorization: `Bearer ${account.accessToken}`,
          [OBSERVATION_CREDENTIAL_HEADER]: config.credential,
          "content-type": "application/json",
        },
        body,
        signal: AbortSignal.timeout(ATTEMPT_TIMEOUT_MS),
      });
    } catch {
      continue;
    }
    if (response.status >= 500) continue;
    if (!response.ok) return { skipped: `core refused (${response.status})` };
    try {
      const parsed = (await response.json()) as {
        version?: unknown;
        operationId?: unknown;
        outcome?: unknown;
      };
      if (
        parsed.version === OBSERVATION_VERSION &&
        parsed.operationId === operationId &&
        (parsed.outcome === "recorded" || parsed.outcome === "unchanged")
      ) {
        return { outcome: parsed.outcome };
      }
    } catch {
      // Fall through.
    }
    return { skipped: "core response invalid" };
  }
  return { skipped: "core unavailable" };
}

/// Production wiring: identify through Hosted Device, post with global fetch.
export function hostedObservationDependencies(
  device: HostedDeviceConfig,
  account: AccountAuthContext,
  brainPublicOrigin: string
): Dependencies {
  return {
    identifyMember: () =>
      hostedDeviceBrainIdentityProvider(
        device,
        account,
        { version: "finite-brain-identity-provider-v1", operation: "identifyMember", input: {} },
        brainPublicOrigin
      ),
    fetch,
    now: () => new Date(),
  };
}
