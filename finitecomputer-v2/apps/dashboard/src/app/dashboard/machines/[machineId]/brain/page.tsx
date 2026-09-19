import { dashboardAgentDesignPreviewEnabled } from "@/lib/dashboard-design-preview";
import headingStyles from "@/styles/agent-page-heading.module.css";
import { redirect } from "next/navigation";
import { BrainChatState } from "@/components/brain-chat-state";
import "@/styles/brain-overview.css";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

export default async function MachineBrainPage({ params }: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
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
  // FIN-89 owns the real agent-authorized membership read. Missing integration
  // must never imply that this agent has no brains or particular permissions.
  return (
    <section className={`brain-view ${headingStyles.page}`} aria-label="Brain">
      <header className="brain-heading">
        <h1 className={headingStyles.title}>Brain</h1>
        <p className={headingStyles.subtitle}>Explore knowledge with {access.displayName}</p>
      </header>
      <BrainChatState key={access.machineId} agentName={access.displayName} machineId={access.machineId} unavailable />
    </section>
  );
}
