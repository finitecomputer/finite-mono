import { dashboardAgentDesignPreviewEnabled } from "@/lib/dashboard-design-preview";
import headingStyles from "@/styles/agent-page-heading.module.css";
import { redirect } from "next/navigation";
import { AgentBrainBrowser } from "@/components/agent-brain-browser";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import "@/styles/brain-overview.css";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

export const dynamic = "force-dynamic";

export default async function MachineBrainPage({ params }: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  const [access, account] = await Promise.all([loadDashboardMachineAccess(machineId), getAccountAuthContext()]);
  if (!access) redirect("/dashboard");
  if (access.machineId !== machineId) {
    redirect(`/dashboard/machines/${encodeURIComponent(access.machineId)}/brain`);
  }
  if (dashboardAgentDesignPreviewEnabled(access.machineId)) {
    const { BrainOverviewTable } = await import("@/components/brain-overview-table");
    return (
      <section className={`brain-view ${headingStyles.page}`} aria-label="Brain">
        <BrainOverviewTable key={access.machineId} agentName={access.displayName} machineId={access.machineId} />
      </section>
    );
  }
  const scope = JSON.stringify([account.workosUserId ?? account.email, account.organizationId ?? null, access.machineId]);
  return (
    <section className={`brain-view ${headingStyles.page}`} aria-label="Brain">
      <AgentBrainBrowser key={scope} runtimeId={access.machineId} agentName={access.displayName} />
    </section>
  );
}
