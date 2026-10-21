// Front door: the legacy Finite Computer dashboard hands signed-in accounts
// to the new Finite product (finite.chat).
//
// Everything here sits behind FC_FRONT_DOOR_ENABLED. Disabled (the default),
// or enabled with an invalid destination, the dashboard behaves exactly as it
// did before this module existed. Every uncertain input keeps the account on
// the legacy dashboard: nobody may lose access to an agent because Core was
// slow, a response was malformed, or a status is unfamiliar.
//
// The redirect is decided only by the /dashboard layout, which Next.js applies
// to dashboard pages and never to route handlers. API routes, /callback,
// /logout, /site-auth, /google-workspace/*, the Stripe webhook, Core's
// /internal/* edge routes and static assets are therefore never redirected.

import {
  coreProductProjects,
  type CoreAgentCreationRequestSummary,
  type CoreBillingOverview,
  type CoreMe,
} from "./core-client";
import { workosBaseUrl } from "./workos-auth";

export const FRONT_DOOR_DEFAULT_TARGET_URL = "https://finite.chat/";
export const FRONT_DOOR_DEFAULT_SIGNUP_URL = "https://finite.chat/auth/login";

/**
 * Stable, documented way back to the legacy dashboard (finite.chat and the
 * import flow link here). GET opens this dashboard for the browser session
 * even when "use the new Finite by default" is on and lands on the setting;
 * POST changes the setting. It is a route handler, so it is never redirected.
 */
export const FRONT_DOOR_SETTINGS_PATH = "/dashboard/front-door";
/** Where the setting renders on the dashboard home. */
export const FRONT_DOOR_SETTINGS_ANCHOR = "front-door";
/** Session cookie set by GET FRONT_DOOR_SETTINGS_PATH: "stay on legacy for now". */
export const FRONT_DOOR_STAY_COOKIE = "finite-front-door-stay";
export const FRONT_DOOR_STAY_SECONDS = 12 * 60 * 60;
/**
 * Set by POST FRONT_DOOR_SETTINGS_PATH after the preference changes. While
 * present the layout reads `/me` uncached: its server SWR entry can lag up to
 * 30s, and a route handler's cache invalidation need not reach the layout.
 */
export const FRONT_DOOR_FRESH_COOKIE = "finite-front-door-fresh";
export const FRONT_DOOR_FRESH_SECONDS = 30;

type EnvSource = Record<string, string | undefined>;

export type FrontDoorConfig =
  | { enabled: false; invalid: string[] }
  | { enabled: true; targetUrl: string; signupUrl: string };

export type FrontDoorStayReason =
  | "disabled"
  | "admin"
  | "legacy-visit"
  | "operator"
  | "account-unavailable"
  | "legacy-billing"
  | "legacy-agent"
  | "paid-setup";

export type FrontDoorDecision =
  | { kind: "stay"; reason: FrontDoorStayReason }
  | { kind: "redirect"; reason: "migrated" | "no-legacy-agent"; url: string };

export type FrontDoorAccount = {
  /** Operator (dashboard admin) accounts always keep the legacy dashboard. */
  isAdmin: boolean;
  /** This browser opened FRONT_DOOR_SETTINGS_PATH to use the legacy dashboard. */
  visitingLegacy: boolean;
  /** The signed-in WorkOS user, when known. */
  workosUserId: string | null;
  /** Core `/me` for this request; null when it was not loaded. */
  me: CoreMe | null;
  /** Core is unconfigured or `/me` failed for this request. */
  coreUnavailable: boolean;
  /** Core billing overview for this request; null when it was not loaded. */
  billing: CoreBillingOverview | null;
  /** Core is unconfigured or the billing read failed for this request. */
  billingUnavailable: boolean;
  /** A Stripe checkout for a legacy agent started before the switch flipped. */
  paidSetupPending: boolean;
};

/** Checks made only before an account would be sent away. */
export type FrontDoorConfirmation = {
  /** An uncached Core `/me`; the layout's own read may be up to 30s stale. */
  reloadAccount: () => Promise<Pick<FrontDoorAccount, "me" | "coreUnavailable">>;
  /** Operator-organization membership regardless of this session's org. */
  isOperatorMember: () => Promise<boolean>;
};

/** Stripe subscription states after which a legacy subscription stops billing. */
const ENDED_SUBSCRIPTION_STATUSES = new Set<string>(["canceled", "incomplete_expired"]);

export function frontDoorConfig(env: EnvSource = process.env): FrontDoorConfig {
  if ((env.FC_FRONT_DOOR_ENABLED ?? "").trim().toLowerCase() !== "true") {
    return { enabled: false, invalid: [] };
  }
  const ownOrigins = dashboardOrigins(env);
  const targetUrl = destinationUrl(
    env.FC_FRONT_DOOR_TARGET_URL,
    FRONT_DOOR_DEFAULT_TARGET_URL,
    ownOrigins
  );
  const signupUrl = destinationUrl(
    env.FC_FRONT_DOOR_SIGNUP_URL,
    FRONT_DOOR_DEFAULT_SIGNUP_URL,
    ownOrigins
  );
  if (!targetUrl || !signupUrl) {
    return {
      enabled: false,
      invalid: [
        ...(targetUrl ? [] : ["FC_FRONT_DOOR_TARGET_URL"]),
        ...(signupUrl ? [] : ["FC_FRONT_DOOR_SIGNUP_URL"]),
      ],
    };
  }
  return { enabled: true, targetUrl, signupUrl };
}

let reportedInvalidConfig = false;

/** Process configuration, reporting an invalid enabled switch once. */
export function loadFrontDoorConfig(): FrontDoorConfig {
  const config = frontDoorConfig();
  if (!config.enabled && config.invalid.length > 0 && !reportedInvalidConfig) {
    reportedInvalidConfig = true;
    console.error(
      "[front-door] FC_FRONT_DOOR_ENABLED is set but the destination is invalid; the front door stays off",
      { invalid: config.invalid }
    );
  }
  return config;
}

/**
 * The legacy dashboard offers and accepts new agents while the switch is off,
 * and always for operators (rollouts create throwaway fresh agents). Whether
 * other legacy users keep creation is an open product decision; today they
 * do not.
 */
export function frontDoorOffersAgentCreation(config: FrontDoorConfig, isAdmin: boolean) {
  return !config.enabled || isAdmin;
}

export function frontDoorAgentCreationMovedMessage(config: FrontDoorConfig) {
  const host = config.enabled ? new URL(config.targetUrl).host : "finite.chat";
  return `New agents are now created in the new Finite at ${host}. Your existing agents stay available here.`;
}

export function frontDoorDecision(
  config: FrontDoorConfig,
  account: FrontDoorAccount
): FrontDoorDecision {
  if (!config.enabled) return { kind: "stay", reason: "disabled" };
  // Admin pages live on this dashboard; operators are never sent away.
  if (account.isAdmin) return { kind: "stay", reason: "admin" };
  // An explicit "take me back" visit wins for this browser session, so a
  // user who opted into the new Finite can always reach the setting again.
  if (account.visitingLegacy) return { kind: "stay", reason: "legacy-visit" };
  const me = account.me;
  if (
    account.coreUnavailable ||
    !me ||
    !account.workosUserId ||
    me.workos_user_id !== account.workosUserId ||
    account.billingUnavailable ||
    !account.billing
  ) {
    return { kind: "stay", reason: "account-unavailable" };
  }
  // The Stripe portal ("Manage billing") lives only on this dashboard, and
  // removing an agent does not cancel its subscription. Anyone still billed
  // for the legacy product stays, migrated or not, until it ends.
  if (legacySubscriptionNeedsDashboard(account.billing)) {
    return { kind: "stay", reason: "legacy-billing" };
  }
  if (accountMigrated(me)) {
    return { kind: "redirect", reason: "migrated", url: config.targetUrl };
  }
  if (hasLegacyAgent(me)) return { kind: "stay", reason: "legacy-agent" };
  if (account.paidSetupPending) return { kind: "stay", reason: "paid-setup" };
  return { kind: "redirect", reason: "no-legacy-agent", url: config.targetUrl };
}

/**
 * Decide with the layout's cached reads, and confirm before ever sending an
 * account away: a fresh `/me` (a cached read can predate a just-completed
 * paid launch), then operator membership in any session org. Any failure
 * while confirming keeps the account here.
 */
export async function resolveFrontDoorRoute(
  config: FrontDoorConfig,
  account: FrontDoorAccount,
  confirm: FrontDoorConfirmation
): Promise<FrontDoorDecision> {
  const cached = frontDoorDecision(config, account);
  if (cached.kind !== "redirect") return cached;
  let decision: FrontDoorDecision;
  try {
    decision = frontDoorDecision(config, { ...account, ...(await confirm.reloadAccount()) });
  } catch {
    return { kind: "stay", reason: "account-unavailable" };
  }
  if (decision.kind !== "redirect") return decision;
  try {
    if (await confirm.isOperatorMember()) return { kind: "stay", reason: "operator" };
  } catch {
    return { kind: "stay", reason: "account-unavailable" };
  }
  return decision;
}

/**
 * True while a legacy Stripe subscription can still bill: any subscription
 * whose status is not `canceled` or `incomplete_expired` (unknown or missing
 * statuses included).
 */
export function legacySubscriptionNeedsDashboard(
  billing: Pick<CoreBillingOverview, "billing_account">
) {
  const subscription = billing.billing_account;
  if (!subscription?.stripe_subscription_id?.trim()) return false;
  return !ENDED_SUBSCRIPTION_STATUSES.has(subscription.subscription_status ?? "");
}

/** The "Try the new Finite" banner is shown only to accounts that stay. */
export function frontDoorBannerHref(
  config: FrontDoorConfig,
  decision: FrontDoorDecision
) {
  return config.enabled && decision.kind === "stay" ? config.targetUrl : null;
}

/** The user turned on "use the new Finite by default" (Core `migrated_at`). */
export function accountMigrated(me: Pick<CoreMe, "migrated_at">) {
  return typeof me.migrated_at === "string" && me.migrated_at.trim() !== "";
}

/**
 * An account has a legacy agent when Core's `/me` shows either
 *
 * - a product Project with an active runtime link (`runtime` is present only
 *   while `project_runtime_links.active`; retired agents' Projects are not
 *   listed at all), or
 * - an agent creation request still in flight: `requested` or `launching`
 *   (Core's own in-flight predicate), a `running` request not yet assigned a
 *   runtime, or any status this dashboard does not recognise.
 *
 * `failed` and `cancelled` requests, and `running` requests whose runtime has
 * since been retired, are not agents. A malformed response counts as an agent.
 */
export function hasLegacyAgent(
  me: Pick<CoreMe, "projects" | "agent_creation_requests">
) {
  if (!Array.isArray(me.projects) || !Array.isArray(me.agent_creation_requests)) {
    return true;
  }
  if (coreProductProjects(me.projects).some((project) => project.runtime != null)) {
    return true;
  }
  return me.agent_creation_requests.some(creationRequestKeepsLegacy);
}

function creationRequestKeepsLegacy(request: CoreAgentCreationRequestSummary) {
  switch (request.status) {
    case "failed":
    case "cancelled":
      return false;
    case "running":
      return !request.agent_runtime_id?.trim();
    default:
      return true;
  }
}

function dashboardOrigins(env: EnvSource) {
  const origins = new Set<string>();
  for (const value of [
    workosBaseUrl(env),
    env.FC_DASHBOARD_BASE_URL,
    env.NEXT_PUBLIC_APP_URL,
  ]) {
    const origin = parsedUrl(value)?.origin;
    if (origin) origins.add(origin);
  }
  return origins;
}

/**
 * An absolute https destination off this dashboard's own origin. Plain http is
 * accepted only for loopback development hosts. A destination on the
 * dashboard's origin could loop, so it disables the front door instead.
 */
function destinationUrl(
  value: string | undefined,
  fallback: string,
  ownOrigins: Set<string>
) {
  const url = parsedUrl(value?.trim() || fallback);
  if (!url || url.username || url.password) return null;
  const loopback = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  if (url.protocol !== "https:" && !(url.protocol === "http:" && loopback)) {
    return null;
  }
  if (ownOrigins.has(url.origin)) return null;
  return url.toString();
}

function parsedUrl(value: string | null | undefined) {
  if (!value?.trim()) return null;
  try {
    return new URL(value.trim());
  } catch {
    return null;
  }
}
