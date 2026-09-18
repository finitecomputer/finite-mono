import headingStyles from "@/styles/agent-page-heading.module.css";
import { redirect } from "next/navigation";

import { ConnectionsPanel } from "@/components/connections-panel";
import { PageHeader } from "@/components/page-header";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import { googleWorkspaceOAuthConfigured } from "@/lib/google-workspace-oauth";

export default async function MachineConnectionsPage({
  params,
}: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access) redirect("/dashboard");
  if (access.machineId !== machineId) {
    redirect(
      `/dashboard/machines/${encodeURIComponent(access.machineId)}/connections`
    );
  }

  return (
    <div className={`${headingStyles.page} space-y-6`}>
      <PageHeader hierarchy="agent" title="Connections" description={`Choose how ${access.displayName} works with you.`} />
      <ConnectionsPanel
        machineId={access.machineId}
        googleConfigured={googleWorkspaceOAuthConfigured()}
      />
    </div>
  );
}
