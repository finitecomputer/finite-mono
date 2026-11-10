/**
 * FIN-463: finite.computer's people move to finite.chat.
 *
 * - Phase 1: once Core says every agent a person had has moved
 *   (`moved_to_finite_chat` on `/me`), the dashboard sends them there.
 * - Phase 2: one switch, `FC_REDIRECT_ALL_TO_FINITE_CHAT`, sends every page
 *   visit there, signed in or not. Machine routes keep answering.
 *
 * finite.chat opens agents by their fragment, not by npub, so both go to
 * its front page. Delete with the rest of the move.
 */

type EnvSource = Record<string, string | undefined>;

const TRUTHY_ENV_VALUES = new Set(["1", "true", "yes", "on"]);

export const FINITE_CHAT_URL = "https://finite.chat/";

/**
 * Paths that keep answering in phase 2: Core's and the runner's API calls,
 * webhooks, health checks, and the sign-in callbacks a connection may still
 * be finishing.
 */
const PHASE_TWO_STAYS = ["/api", "/health", "/healthz", "/callback", "/site-auth", "/google-workspace"];

/** Where people are sent: `FC_FINITE_CHAT_URL`, or finite.chat itself. */
export function finiteChatUrl(env: EnvSource = process.env) {
  const configured = env.FC_FINITE_CHAT_URL?.trim();
  if (!configured) return FINITE_CHAT_URL;
  const url = URL.parse(configured);
  if (!url || !["https:", "http:"].includes(url.protocol)) {
    throw new Error("FC_FINITE_CHAT_URL is not an http(s) URL");
  }
  return url.toString();
}

export function redirectAllToFiniteChat(env: EnvSource = process.env) {
  return TRUTHY_ENV_VALUES.has((env.FC_REDIRECT_ALL_TO_FINITE_CHAT ?? "").trim().toLowerCase());
}

/** Phase 2: where this request goes instead, or null to serve it. */
export function phaseTwoRedirect(pathname: string, method: string, env: EnvSource = process.env) {
  if (!redirectAllToFiniteChat(env)) return null;
  if (!["GET", "HEAD"].includes(method.toUpperCase())) return null;
  if (PHASE_TWO_STAYS.some((prefix) => pathname === prefix || pathname.startsWith(`${prefix}/`))) return null;
  return finiteChatUrl(env);
}

/** Phase 1: where a person whose agents have all moved goes, or null. */
export function movedRedirect(me: { moved_to_finite_chat?: boolean } | null | undefined, env: EnvSource = process.env) {
  return me?.moved_to_finite_chat === true ? finiteChatUrl(env) : null;
}
