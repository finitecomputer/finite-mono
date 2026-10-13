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
  frontDoorBannerHref,
  frontDoorDecision,
  frontDoorOffersAgentCreation,
  loadFrontDoorConfig,
} from "@/lib/front-door";

export default async function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const [viewer, core, billing] = await Promise.all([
    loadOptionalViewerContext(),
    loadCoreMe({ cacheMode: "swr" }),
    loadCoreBillingOverview({ cacheMode: "fresh" }),
  ]);
  // The front door is decided here only: Next.js applies this layout to
  // dashboard pages, never to route handlers, so no API, callback, webhook
  // or asset request is ever redirected.
  const frontDoor = loadFrontDoorConfig();
  const frontDoorRoute = frontDoorDecision(frontDoor, {
    isAdmin: viewer.isAdmin,
    workosUserId: core.account.workosUserId,
    me: core.me,
    coreUnavailable: !core.configured || Boolean(core.error),
    paidSetupPending:
      frontDoor.enabled &&
      draftStartedStripeCheckout(
        await unsealAgentOnboardingDraft(
          (await cookies()).get(AGENT_DRAFT_COOKIE)?.value,
          core.account.workosUserId
        )
      ),
  });
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
      agentCreationOffered={frontDoorOffersAgentCreation(frontDoor)}
      firstAgentAccount={Boolean(core.me && !core.error && coreProductProjects(core.me.projects).length === 0)}
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
