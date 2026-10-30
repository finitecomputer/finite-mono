import { redirect } from "next/navigation";
import { AgentExportPanel } from "@/components/agent-export-panel";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

export const dynamic = "force-dynamic";

export default async function MachineExportPage({ params }: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  const [access, account] = await Promise.all([loadDashboardMachineAccess(machineId), getAccountAuthContext()]);
  if (!access) redirect("/dashboard");
  if (access.machineId !== machineId) {
    redirect(`/dashboard/machines/${encodeURIComponent(access.machineId)}/export`);
  }
  const scope = JSON.stringify([account.workosUserId ?? account.email, account.organizationId ?? null, access.machineId]);
  return <AgentExportPanel key={scope} runtimeId={access.machineId} agentName={access.displayName} />;
}
