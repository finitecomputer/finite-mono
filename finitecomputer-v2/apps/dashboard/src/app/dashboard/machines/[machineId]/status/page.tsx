import { notFound } from "next/navigation";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import { AgentStatusPanel } from "@/components/agent-status-panel";

export default async function AgentStatusPage({ params }: { params: Promise<{ machineId: string }> }) {
  const { machineId } = await params;
  const access = await loadDashboardMachineAccess(machineId);
  if (!access?.viewer.isAdmin) notFound();
  return <AgentStatusPanel projectId={access.coreProject.project.id} />;
}
