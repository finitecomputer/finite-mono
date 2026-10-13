import { loadCoreRequesterEmail } from "@/lib/core-client";
import type { AccountAuthContext } from "@/lib/dashboard-auth";
import type { HostedRequesterContext } from "@/lib/hosted-web-device";
import { sitesUpstreamOrigin } from "@/lib/site-preview";

const HOSTED_REQUESTER_ASSERTION_TIMEOUT_MS = 5_000;

export type HostedRequesterSubjects = {
  /** The human's Finite Chat account (hex or npub). */
  requesterNpub: string;
  /** The Agent Principal verified for this exact Core-authorized Project. */
  agentNpub: string;
};

/**
 * Optional Sites requester context for a text send. Requester assertions and
 * publishing grants belong to the same Sites registry. Every failure returns
 * undefined: Chat stays available if Sites is down, and Project Init returns
 * the structured requester_email_required response so the skill can ask.
 */
export async function hostedRequesterAssertion(
  account: AccountAuthContext,
  subjects: () => Promise<HostedRequesterSubjects | undefined>
): Promise<HostedRequesterContext | undefined> {
  const upstream = sitesUpstreamOrigin();
  const serviceToken = process.env.FINITE_SITES_VIEWER_SESSION_TOKEN?.trim();
  if (!account.workosUserId || !account.emailVerified || !upstream || !serviceToken) {
    return undefined;
  }
  try {
    const email = await loadCoreRequesterEmail(account);
    const subject = await subjects();
    if (!subject) return undefined;
    const response = await fetch(
      `${upstream}/internal/v1/hosted-requester-assertions`,
      {
        method: "POST",
        redirect: "error",
        headers: {
          authorization: `Bearer ${serviceToken}`,
          "content-type": "application/json",
        },
        body: JSON.stringify({
          email,
          requester_npub: subject.requesterNpub,
          agent_npub: subject.agentNpub,
        }),
        cache: "no-store",
        signal: AbortSignal.timeout(HOSTED_REQUESTER_ASSERTION_TIMEOUT_MS),
      }
    );
    if (!response.ok) return undefined;
    const payload = (await response.json()) as {
      email?: unknown;
      assertion?: unknown;
    };
    if (
      payload.email !== email ||
      typeof payload.assertion !== "string" ||
      !payload.assertion
    ) {
      return undefined;
    }
    return { email, sitesAssertion: payload.assertion };
  } catch {
    return undefined;
  }
}
