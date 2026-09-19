import { dashboardAgentDesignPreviewEnabled } from "@/lib/dashboard-design-preview";
import { redirect } from "next/navigation";

import { AgentSitesBrowser } from "@/components/agent-sites-browser";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

export default async function SitesPage({ params, searchParams }: {
  params: Promise<{ machineId: string }>;
  searchParams: Promise<{ preview?: string; state?: string }>;
}) {
  const { machineId } = await params;
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access) redirect("/dashboard");
  if (access.machineId !== machineId) {
    redirect(`/dashboard/machines/${encodeURIComponent(access.machineId)}/sites`);
  }

  // Account-wide listing and access metadata await the Sites service contract.
  // Never infer ownership or sharing permissions from chat links or this agent.
  const query = await searchParams;
  if (dashboardAgentDesignPreviewEnabled(access.machineId)) {
    const { sampleSites } = await import("@/components/dev/sites-sample");
    return <>
      <p className="mb-5 text-sm text-muted-foreground">Design preview · Sample Sites records</p>
      <AgentSitesBrowser key={access.machineId} machineId={access.machineId} sites={query.state === "empty" ? [] : query.state === "unavailable" ? null : sampleSites} />
    </>;
  }
  return <AgentSitesBrowser key={access.machineId} machineId={access.machineId} sites={null} listingNotConnected />;
}
