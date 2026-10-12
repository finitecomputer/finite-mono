import { sealData, unsealData } from "iron-session";
import type { AgentHostingTier } from "./agent-onboarding";

export const LAUNCH_INVITATION_COOKIE = "finite-launch-invitation";
export const LAUNCH_INVITATION_TTL = 30 * 24 * 60 * 60;
type Env = Record<string, string | undefined>;
export type LaunchInvitation = {
  purpose: "organization-seat-v1";
  email: string;
  organization: string;
  launchCode: string;
  hostingTier: AgentHostingTier;
  expiresAtMs: number;
};

export function invitationRecipients(value: FormDataEntryValue | null, count: number) {
  const text = typeof value === "string" ? value.trim() : "";
  if (!text) return [];
  const emails = text.split(/[\n,;]+/u).map(email => email.trim().toLowerCase());
  if (emails.some(email => email.length > 254 || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/u.test(email))) {
    throw new Error("Enter one valid recipient email per seat.");
  }
  if (emails.length !== count || new Set(emails).size !== count) {
    throw new Error("Enter one different recipient email for each code in the batch.");
  }
  return emails;
}

export function invitationPassword(env: Env = process.env) {
  const password = env.WORKOS_COOKIE_PASSWORD?.trim();
  if (!password || password.length < 32) throw new Error("Invitation links are unavailable.");
  return password;
}

/** Encrypt the code as well as authenticating its recipient; never put a raw code in a link. */
export async function sealLaunchInvitation(invitation: LaunchInvitation, env: Env = process.env) {
  return sealData(invitation, { password: invitationPassword(env), ttl: LAUNCH_INVITATION_TTL });
}

export async function readLaunchInvitation(
  token: string | undefined | null,
  env: Env = process.env,
  options: { nowMs?: number; allowExpiredRedemption?: boolean } = {},
): Promise<LaunchInvitation | null> {
  const now = options.nowMs ?? Date.now();
  if (!token || token.length > 3800) return null;
  try {
    const invitation = await unsealData<LaunchInvitation>(token, { password: invitationPassword(env) });
    if (invitation.purpose !== "organization-seat-v1" ||
        typeof invitation.email !== "string" || !invitation.email ||
        typeof invitation.organization !== "string" || !invitation.organization ||
        typeof invitation.launchCode !== "string" || !invitation.launchCode ||
        !["standard", "confidential"].includes(invitation.hostingTier) ||
        !Number.isFinite(invitation.expiresAtMs) || (!options.allowExpiredRedemption && invitation.expiresAtMs <= now)) return null;
    return invitation;
  } catch { return null; }
}

export function requireInvitationAccount(invitation: LaunchInvitation, account: { email: string | null; emailVerified: boolean; workosUserId: string | null }) {
  if (!account.workosUserId || !account.emailVerified || account.email?.trim().toLowerCase() !== invitation.email) {
    throw new Error("Make sure you are signing in with the correct email provided to you by your organization.");
  }
}
