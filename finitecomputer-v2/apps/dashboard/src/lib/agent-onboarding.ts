import { sealData, unsealData } from "iron-session";

import type { CoreRunnerClass } from "@/lib/core-client";

export const AGENT_DRAFT_COOKIE = "finite-agent-draft";
export const AGENT_DRAFT_TTL_SECONDS = 24 * 60 * 60;
export const MAX_AGENT_PROFILE_IMAGE_BYTES = 5 * 1024 * 1024;

const AGENT_CREATION_ENTITLEMENT_EXHAUSTED = "agent creation entitlement is exhausted";

const RUNNER_CLASSES = new Set<CoreRunnerClass>([
  "local_docker",
  "apple_container",
  "kata",
  "phala",
  "enclavia",
]);

export type AgentOnboardingDraft = {
  version: 1;
  workosUserId: string;
  displayName: string;
  profilePictureUrl: string | null;
  runnerClass: CoreRunnerClass;
  idempotencyKey: string;
  issuedAtMs: number;
};

export type AgentCreationAccessPath =
  | "launch-code"
  | "stripe"
  | "entitlement"
  | "denied";

/** Keep each onboarding submit on the access path the person explicitly chose. */
export function resolveAgentCreationAccessPath(
  access: FormDataEntryValue | null,
  canCreateAgent: boolean
): AgentCreationAccessPath {
  if (access === "launch-code") return "launch-code";
  if (access === "stripe") return "stripe";
  if (access === "entitled" && canCreateAgent) return "entitlement";
  return "denied";
}

export function agentCreationErrorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : "Could not create agent.";
  if (message.toLowerCase().includes(AGENT_CREATION_ENTITLEMENT_EXHAUSTED)) {
    return "This account already has an agent. Open it from your dashboard, or ask an operator to remove it before creating another.";
  }
  return message;
}

export function configuredRunnerClasses(
  env: Record<string, string | undefined> = process.env
): CoreRunnerClass[] {
  const fallback = defaultRunnerClass(env);
  const configured = (env.FC_DASHBOARD_RUNNER_CLASSES ?? fallback)
    .split(",")
    .map((value) => value.trim())
    .filter((value): value is CoreRunnerClass => RUNNER_CLASSES.has(value as CoreRunnerClass));
  return configured.length ? Array.from(new Set(configured)) : [fallback];
}

export function defaultRunnerClass(
  env: Record<string, string | undefined> = process.env
): CoreRunnerClass {
  const configured = env.FC_DASHBOARD_DEFAULT_RUNNER_CLASS?.trim();
  if (configured && RUNNER_CLASSES.has(configured as CoreRunnerClass)) {
    return configured as CoreRunnerClass;
  }
  return env.NODE_ENV === "production" ? "kata" : "apple_container";
}

export function resolveRunnerClass(
  requested: FormDataEntryValue | null,
  env: Record<string, string | undefined> = process.env
) {
  const allowed = configuredRunnerClasses(env);
  const candidate = typeof requested === "string" ? requested.trim() : "";
  if (!candidate) return allowed[0];
  if (!allowed.includes(candidate as CoreRunnerClass)) {
    throw new Error("That hosting option is not available.");
  }
  return candidate as CoreRunnerClass;
}

export function normalizeAgentDisplayName(value: FormDataEntryValue | null) {
  const name = typeof value === "string" ? value.trim().replace(/\s+/gu, " ") : "";
  if (!name || name.length > 80 || /[\u0000-\u001f\u007f]/u.test(name)) {
    throw new Error("Choose an agent name between 1 and 80 characters.");
  }
  return name;
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
      !RUNNER_CLASSES.has(draft.runnerClass) ||
      !Number.isFinite(draft.issuedAtMs) ||
      draft.issuedAtMs > nowMs + 60_000 ||
      nowMs - draft.issuedAtMs > AGENT_DRAFT_TTL_SECONDS * 1000
    ) {
      return null;
    }
    return draft;
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
