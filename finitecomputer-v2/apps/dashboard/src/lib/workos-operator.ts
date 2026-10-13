// Operator-organization membership regardless of the session's organization.
//
// Dashboard admin is decided by the session's org claim, so an operator signed
// in under another organization is not an admin for that session. The front
// door asks WorkOS directly, and only before it would send an account away, so
// such an operator keeps the legacy dashboard (switching org there restores
// the admin pages). This never grants admin access.

type EnvSource = Record<string, string | undefined>;

export const WORKOS_OPERATOR_LOOKUP_TIMEOUT_MS = 2_000;

type MembershipLister = (options: {
  userId: string;
  organizationId: string;
  statuses: ["active"];
  limit: number;
}) => Promise<{ data: unknown[] }>;

/**
 * Whether the WorkOS user has an active membership in FC_WORKOS_OPERATOR_ORG_ID.
 * Throws when WorkOS cannot answer in time; the caller treats that as unknown.
 */
export async function workosOperatorMember(
  workosUserId: string,
  env: EnvSource = process.env,
  listMemberships?: MembershipLister
) {
  const organizationId = env.FC_WORKOS_OPERATOR_ORG_ID?.trim();
  if (!organizationId || !workosUserId.trim()) return false;
  const list = listMemberships ?? (await workosMembershipLister());
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const memberships = await Promise.race([
      list({ userId: workosUserId, organizationId, statuses: ["active"], limit: 1 }),
      new Promise<never>((_, reject) => {
        timer = setTimeout(
          () => reject(new Error("WorkOS operator lookup timed out")),
          WORKOS_OPERATOR_LOOKUP_TIMEOUT_MS
        );
      }),
    ]);
    return memberships.data.length > 0;
  } finally {
    clearTimeout(timer);
  }
}

async function workosMembershipLister(): Promise<MembershipLister> {
  const { getWorkOS } = await import("@workos-inc/authkit-nextjs");
  const userManagement = getWorkOS().userManagement;
  return (options) => userManagement.listOrganizationMemberships(options);
}
