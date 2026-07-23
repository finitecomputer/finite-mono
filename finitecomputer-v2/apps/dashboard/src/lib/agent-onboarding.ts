import { sealData, unsealData } from "iron-session";

export const AGENT_DRAFT_COOKIE = "finite-agent-draft";
export const AGENT_DRAFT_TTL_SECONDS = 24 * 60 * 60;
export const MAX_AGENT_PROFILE_IMAGE_BYTES = 5 * 1024 * 1024;
export type AgentHostingTier = "standard" | "confidential";

const AGENT_CREATION_ENTITLEMENT_EXHAUSTED = "agent creation entitlement is exhausted";
const AGENT_CREATION_BILLING_REQUIRED = "billing is required before creating an agent";
const AGENT_CREATION_HOSTING_TIER_NOT_AUTHORIZED =
  "selected hosting tier is not authorized by this account or launch code";
const AGENT_CREATION_BILLING_REQUIRED_MESSAGE =
  "Choose payment or enter a Launch Code to continue.";
const AGENT_CREATION_HOSTING_TIER_MESSAGE =
  "This account or Launch Code does not match the selected hosting option. Choose the matching option or use a different Launch Code.";

export type AgentCreationRecovery = "access" | null;

export type AgentOnboardingDraft = {
  version: 1;
  workosUserId: string;
  displayName: string;
  profilePictureUrl: string | null;
  hostingTier: AgentHostingTier;
  idempotencyKey: string;
  issuedAtMs: number;
  /** Validated Runtime id used only to return to the originating agent. */
  returnMachineId?: string | null;
  /** Present only after this signed draft actually initiated Stripe Checkout. */
  stripeCheckoutStartedAtMs?: number | null;
};

export type AgentCreationAccessPath =
  | "launch-code"
  | "stripe"
  | "entitlement"
  | "denied";

/** Keep each onboarding submit on the access path the person explicitly chose. */
export function resolveAgentCreationAccessPath(
  access: FormDataEntryValue | null,
  canCreateAgent: boolean,
  allowExistingEntitlement = true
): AgentCreationAccessPath {
  if (access === "launch-code") return "launch-code";
  if (access === "stripe") return "stripe";
  if (access === "entitled" && canCreateAgent && allowExistingEntitlement) {
    return "entitlement";
  }
  return "denied";
}

export function draftStartedStripeCheckout(
  draft: AgentOnboardingDraft | null
): draft is AgentOnboardingDraft & { stripeCheckoutStartedAtMs: number } {
  return Boolean(draft && Number.isFinite(draft.stripeCheckoutStartedAtMs));
}

export function agentCreationRequiresAccess({
  runtimeMode,
  canCreateAgent,
  requiresBilling,
  recovery,
}: {
  runtimeMode: string | undefined;
  canCreateAgent: boolean;
  requiresBilling: boolean;
  recovery?: AgentCreationRecovery;
}) {
  return (
    runtimeMode === "canary" ||
    runtimeMode === "customer" ||
    requiresBilling ||
    !canCreateAgent ||
    recovery === "access"
  );
}

export function agentCreationErrorRecovery(error: unknown): AgentCreationRecovery {
  const message = error instanceof Error ? error.message : String(error ?? "");
  const normalized = message.toLowerCase();
  return normalized.includes(AGENT_CREATION_BILLING_REQUIRED) ||
    normalized === AGENT_CREATION_BILLING_REQUIRED_MESSAGE.toLowerCase() ||
    normalized.includes(AGENT_CREATION_HOSTING_TIER_NOT_AUTHORIZED)
    ? "access"
    : null;
}

export function agentCreationErrorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : "Could not create agent.";
  if (message.toLowerCase().includes(AGENT_CREATION_ENTITLEMENT_EXHAUSTED)) {
    return "This account already has an agent. Open it from your dashboard, or ask an operator to remove it before creating another.";
  }
  if (message.toLowerCase().includes(AGENT_CREATION_BILLING_REQUIRED)) {
    return AGENT_CREATION_BILLING_REQUIRED_MESSAGE;
  }
  if (message.toLowerCase().includes(AGENT_CREATION_HOSTING_TIER_NOT_AUTHORIZED)) {
    return AGENT_CREATION_HOSTING_TIER_MESSAGE;
  }
  return message;
}

export function normalizeAgentDisplayName(value: FormDataEntryValue | null) {
  const name = typeof value === "string" ? value.trim().replace(/\s+/gu, " ") : "";
  if (!name || name.length > 80 || /[\u0000-\u001f\u007f]/u.test(name)) {
    throw new Error("Choose an agent name between 1 and 80 characters.");
  }
  return name;
}

export function normalizeAgentHostingTier(
  value: FormDataEntryValue | null
): AgentHostingTier {
  if (value === "standard" || value === "confidential") return value;
  throw new Error("Choose Standard or Confidential hosting.");
}

export function normalizeAgentReturnMachineId(value: FormDataEntryValue | null) {
  const machineId = typeof value === "string" ? value.trim() : "";
  return /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u.test(machineId) ? machineId : null;
}

export async function sealAgentOnboardingDraft(
  draft: AgentOnboardingDraft,
  env: Record<string, string | undefined> = process.env
) {
  return sealData(draft, {
    password: draftPassword(env),
    ttl: AGENT_DRAFT_TTL_SECONDS,
  });
}

export async function unsealAgentOnboardingDraft(
  sealed: string | null | undefined,
  workosUserId: string | null | undefined,
  env: Record<string, string | undefined> = process.env,
  nowMs = Date.now()
): Promise<AgentOnboardingDraft | null> {
  if (!sealed || !workosUserId) return null;
  try {
    const draft = await unsealData<AgentOnboardingDraft>(sealed, {
      password: draftPassword(env),
    });
    if (
      draft.version !== 1 ||
      draft.workosUserId !== workosUserId ||
      !draft.displayName?.trim() ||
      (draft.hostingTier != null &&
        draft.hostingTier !== "standard" &&
        draft.hostingTier !== "confidential") ||
      !draft.idempotencyKey?.trim() ||
      !Number.isFinite(draft.issuedAtMs) ||
      (draft.stripeCheckoutStartedAtMs != null &&
        (!Number.isFinite(draft.stripeCheckoutStartedAtMs) ||
          draft.stripeCheckoutStartedAtMs < draft.issuedAtMs ||
          draft.stripeCheckoutStartedAtMs > nowMs + 60_000)) ||
      draft.issuedAtMs > nowMs + 60_000 ||
      nowMs - draft.issuedAtMs > AGENT_DRAFT_TTL_SECONDS * 1000
    ) {
      return null;
    }
    return {
      version: 1,
      workosUserId: draft.workosUserId,
      displayName: draft.displayName,
      profilePictureUrl: draft.profilePictureUrl ?? null,
      hostingTier: draft.hostingTier ?? "standard",
      idempotencyKey: draft.idempotencyKey,
      issuedAtMs: draft.issuedAtMs,
      returnMachineId: normalizeAgentReturnMachineId(draft.returnMachineId ?? null),
      stripeCheckoutStartedAtMs: draft.stripeCheckoutStartedAtMs,
    };
  } catch {
    return null;
  }
}

function draftPassword(env: Record<string, string | undefined>) {
  const password = env.WORKOS_COOKIE_PASSWORD?.trim();
  if (!password || password.length < 32) {
    throw new Error("Agent setup is unavailable.");
  }
  return password;
}
