import { redirect } from "next/navigation";

import { ConnectionsPanel } from "@/components/connections-panel";
import { PageHeader } from "@/components/page-header";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import { googleWorkspaceOAuthConfigured } from "@/lib/google-workspace-oauth";

export default async function MachineConnectionsPage({
  params,
  searchParams,
}: {
  params: Promise<{ machineId: string }>;
  searchParams: Promise<{ [key: string]: string | string[] | undefined }>;
}) {
  const { machineId } = await params;
  const query = await searchParams;
  const openRouterResult =
    typeof query.openrouter === "string" && query.openrouter ? query.openrouter : null;
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access) redirect("/dashboard");
  if (access.machineId !== machineId) {
    redirect(
      `/dashboard/machines/${encodeURIComponent(access.machineId)}/connections`
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader title="Connections" description={`Choose how ${access.displayName} works with you.`} />
      <ConnectionsPanel
        machineId={access.machineId}
        googleConfigured={googleWorkspaceOAuthConfigured()}
        openRouterResult={openRouterResult}
      />
    </div>
  );
}
