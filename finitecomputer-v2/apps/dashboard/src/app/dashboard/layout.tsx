import { cookies } from "next/headers";
import { redirect } from "next/navigation";

import { TrialAccessMonitor } from "@/components/trial-access-monitor";
import { DashboardShell } from "@/components/dashboard-shell";
import {
  AGENT_DRAFT_COOKIE,
  draftStartedStripeCheckout,
  unsealAgentOnboardingDraft,
} from "@/lib/agent-onboarding";
import {
  coreProductProjects,
  coreProjectLabel,
  coreProjectPrimaryUrl,
  coreProjectRuntimeId,
  loadCoreMe,
  loadCoreBillingOverview,
} from "@/lib/core-client";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";
import {
  FRONT_DOOR_FRESH_COOKIE,
  FRONT_DOOR_STAY_COOKIE,
  frontDoorBannerHref,
  frontDoorOffersAgentCreation,
  loadFrontDoorConfig,
  resolveFrontDoorRoute,
} from "@/lib/front-door";
import { workosOperatorMember } from "@/lib/workos-operator";

export default async function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  // The front door is decided here only: Next.js applies this layout to
  // dashboard pages, never to route handlers, so no API, callback, webhook
  // or asset request is ever redirected. Switched off, no cookie is read.
  const frontDoor = loadFrontDoorConfig();
  const frontDoorCookies = frontDoor.enabled ? await cookies() : null;
  const preferenceJustChanged =
    frontDoorCookies?.get(FRONT_DOOR_FRESH_COOKIE)?.value === "1";
  const [viewer, core, billing] = await Promise.all([
    loadOptionalViewerContext(),
    loadCoreMe({ cacheMode: preferenceJustChanged ? "fresh" : "swr" }),
    loadCoreBillingOverview({ cacheMode: "fresh" }),
  ]);
  const account = core.account;
  const agentCreationOffered = frontDoorOffersAgentCreation(frontDoor, viewer.isAdmin);
  const paidSetupPending =
    frontDoorCookies !== null &&
    draftStartedStripeCheckout(
      await unsealAgentOnboardingDraft(
        frontDoorCookies.get(AGENT_DRAFT_COOKIE)?.value,
        account.workosUserId
      )
    );
  const visitingLegacy = frontDoorCookies?.get(FRONT_DOOR_STAY_COOKIE)?.value === "1";
  const frontDoorRoute = await resolveFrontDoorRoute(
    frontDoor,
    {
      isAdmin: viewer.isAdmin,
      visitingLegacy,
      workosUserId: account.workosUserId,
      me: core.me,
      coreUnavailable: !core.configured || Boolean(core.error),
      billing: billing.billing,
      billingUnavailable: !billing.configured || Boolean(billing.error),
      paidSetupPending,
    },
    {
      // Only reached when the cached reads say "send away".
      async reloadAccount() {
        const fresh = await loadCoreMe({ cacheMode: "fresh" });
        return { me: fresh.me, coreUnavailable: !fresh.configured || Boolean(fresh.error) };
      },
      isOperatorMember: async () =>
        account.source === "workos" && account.workosUserId
          ? workosOperatorMember(account.workosUserId)
          : false,
    }
  );
  if (frontDoorRoute.kind === "redirect") {
    console.info("[front-door] redirecting account to the new Finite", {
      reason: frontDoorRoute.reason,
    });
    redirect(frontDoorRoute.url);
  }

  const machineIds = new Set<string>();
  const machines = [
    ...coreProductProjects(core.me?.projects ?? []).flatMap((project) => {
      const runtimeId = coreProjectRuntimeId(project);
      if (!runtimeId || machineIds.has(runtimeId)) {
        return [];
      }
      machineIds.add(runtimeId);
      return [
        {
          id: runtimeId,
          ownerLabel: coreProjectLabel(project),
          runtimeStatus: project.runtime?.runtime_status ?? "unknown",
          siteUrl: coreProjectPrimaryUrl(project) ?? undefined,
        },
      ];
    }),
  ];

  return (
    <DashboardShell
      isAdmin={viewer.isAdmin}
      agentCreationOffered={agentCreationOffered}
      // Once creation has moved, an agentless account keeps the ordinary shell
      // (its billing stays reachable) unless it is finishing a paid setup.
      firstAgentAccount={Boolean(
        core.me && !core.error && coreProductProjects(core.me.projects).length === 0 &&
          (agentCreationOffered || paidSetupPending)
      )}
      frontDoorBannerHref={frontDoorBannerHref(frontDoor, frontDoorRoute)}
      machines={machines}
      saasMode={core.configured}
      viewerEmail={viewer.email}
    >
      {billing.billing?.trial_access ? <TrialAccessMonitor blocked={billing.billing.trial_access.blocked} excludeHome /> : null}
      {children}
    </DashboardShell>
  );
}
