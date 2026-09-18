import headingStyles from "@/styles/agent-page-heading.module.css";
import { redirect } from "next/navigation";

import { ConnectionsPanel } from "@/components/connections-panel";
import { HermesAccessConnection } from "@/components/hermes-access-connection";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { PageHeader } from "@/components/page-header";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import { googleWorkspaceOAuthConfigured } from "@/lib/google-workspace-oauth";

export default async function MachineConnectionsPage({
  params,
}: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  const [access, account] = await Promise.all([
    loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" }),
    getAccountAuthContext(),
  ]);
  if (!access) redirect("/dashboard");
  if (access.machineId !== machineId) {
    redirect(
      `/dashboard/machines/${encodeURIComponent(access.machineId)}/connections`
    );
  }

  return (
    <div className={`${headingStyles.page} space-y-6`}>
      <PageHeader hierarchy="agent" title="Connections" description={`Choose how ${access.displayName} works with you.`} />
      <HermesAccessConnection
        key={JSON.stringify([account.workosUserId ?? account.email, account.organizationId ?? null, access.machineId])}
        runtimeId={access.machineId}
      />
      <ConnectionsPanel
        machineId={access.machineId}
        googleConfigured={googleWorkspaceOAuthConfigured()}
      />
    </div>
  );
}
