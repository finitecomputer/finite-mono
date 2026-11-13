import { randomBytes } from "node:crypto";
import { after } from "next/server";
import type { AccountAuthContext } from "./dashboard-auth";
import type { HostedDeviceConfig } from "./hosted-web-device";
import { brainPublicOrigin } from "./brain-hosted-client";
import {
  brainObservationConfig, hostedObservationDependencies, postAccountObservation,
  type BrainObservationConfig, type ObservationDependencies, type ObservationResult,
} from "./brain-identity-observation";

export const HOSTED_OBSERVATION_VERSION = "finite-core-hosted-account-observation-v1";
export const HOSTED_OBSERVATION_PATH = "/api/core/internal/v1/hosted-account-observations";

/** Load the Account's existing key. No Brain, sharing, signing, or key creation. */
export async function observeHostedAccountIdentity(
  config: BrainObservationConfig | null, account: AccountAuthContext, dependencies: ObservationDependencies
): Promise<ObservationResult> {
  if (!config) return { skipped: "not configured" };
  if (!account.accessToken || !account.workosUserId || !account.emailVerified) return { skipped: "no verified account session" };
  let identity: unknown;
  try { identity = await dependencies.identifyMember(); }
  catch { return { skipped: "no hosted identity" }; }
  const key = identity && typeof identity === "object" && "publicKeyHex" in identity ? identity.publicKeyHex : null;
  if (typeof key !== "string" || !/^[0-9a-f]{64}$/.test(key)) return { skipped: "no hosted identity" };
  const body = {
    version: HOSTED_OBSERVATION_VERSION, operationId: `hosted_${randomBytes(16).toString("hex")}`,
    observedAt: dependencies.now().toISOString(), publicKeyHex: key,
  };
  return postAccountObservation(config, account, HOSTED_OBSERVATION_PATH, body, dependencies.fetch);
}

/** Called only after a successful Chat bootstrap. Capture the verified Account
 * before returning; registration failures must never change Chat's response. */
export function scheduleHostedAccountObservation(
  account: AccountAuthContext, device: HostedDeviceConfig,
  schedule: (work: () => Promise<void>) => void = after
) {
  const config = brainObservationConfig();
  const origin = brainPublicOrigin();
  if (!config || !origin || !account.accessToken || !account.workosUserId || !account.emailVerified) return;
  try {
    schedule(async () => {
      try {
        const result = await observeHostedAccountIdentity(config, account,
          hostedObservationDependencies(device, account, origin));
        if ("skipped" in result) console.warn(`Hosted identity observation skipped: ${result.skipped}`);
      } catch {
        console.warn("Hosted identity observation failed");
      }
    });
  } catch {
    console.warn("Hosted identity observation could not be scheduled");
  }
}
