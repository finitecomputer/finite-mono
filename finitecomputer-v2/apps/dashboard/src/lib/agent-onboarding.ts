import { sealData, unsealData } from "iron-session";

export const AGENT_DRAFT_COOKIE = "finite-agent-draft";
export const AGENT_DRAFT_TTL_SECONDS = 24 * 60 * 60;
export const MAX_AGENT_PROFILE_IMAGE_BYTES = 5 * 1024 * 1024;

const AGENT_CREATION_ENTITLEMENT_EXHAUSTED = "agent creation entitlement is exhausted";
const AGENT_CREATION_BILLING_REQUIRED = "billing is required before creating an agent";

export type AgentOnboardingDraft = {
  version: 1;
  workosUserId: string;
  displayName: string;
  profilePictureUrl: string | null;
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
  error,
}: {
  runtimeMode: string | undefined;
  canCreateAgent: boolean;
  requiresBilling: boolean;
  error?: string | null;
}) {
  return (
    runtimeMode === "canary" ||
    requiresBilling ||
    !canCreateAgent ||
    Boolean(error?.toLowerCase().includes(AGENT_CREATION_BILLING_REQUIRED))
  );
}

export function agentCreationErrorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : "Could not create agent.";
  if (message.toLowerCase().includes(AGENT_CREATION_ENTITLEMENT_EXHAUSTED)) {
    return "This account already has an agent. Open it from your dashboard, or ask an operator to remove it before creating another.";
  }
  if (message.toLowerCase().includes(AGENT_CREATION_BILLING_REQUIRED)) {
    return "Choose payment or enter a Launch Code to continue.";
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
