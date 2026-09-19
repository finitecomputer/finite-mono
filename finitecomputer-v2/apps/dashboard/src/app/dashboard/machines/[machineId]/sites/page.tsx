import { dashboardAgentDesignPreviewEnabled } from "@/lib/dashboard-design-preview";
import { redirect } from "next/navigation";

import { AgentSitesInventory } from "@/components/agent-sites-inventory";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { AgentSitesBrowser } from "@/components/agent-sites-browser";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

export const dynamic = "force-dynamic";

export default async function SitesPage({ params, searchParams }: {
  params: Promise<{ machineId: string }>;
  searchParams: Promise<{ preview?: string; state?: string }>;
}) {
  const { machineId } = await params;
  const [access, account] = await Promise.all([loadDashboardMachineAccess(machineId), getAccountAuthContext()]);
  if (!access) redirect("/dashboard");
  if (access.machineId !== machineId) {
    redirect(`/dashboard/machines/${encodeURIComponent(access.machineId)}/sites`);
  }

  const query = await searchParams;
  if (dashboardAgentDesignPreviewEnabled(access.machineId)) {
    const { sampleSites } = await import("@/components/dev/sites-sample");
    return <>
      <p className="mb-5 text-sm text-muted-foreground">Design preview · Sample Sites records</p>
      <AgentSitesBrowser key={access.machineId} machineId={access.machineId} sites={query.state === "empty" ? [] : query.state === "unavailable" ? null : sampleSites} />
    </>;
  }
  const scope = JSON.stringify([account.workosUserId ?? account.email, account.organizationId ?? null, access.machineId]);
  return <AgentSitesInventory key={scope} runtimeId={access.machineId} agentName={access.displayName} />;
}
