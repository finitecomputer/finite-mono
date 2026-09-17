import headingStyles from "@/styles/agent-page-heading.module.css";
import { notFound, redirect } from "next/navigation";
import { BrainOverviewTable } from "@/components/brain-overview-table";
import "@/styles/brain-overview.css";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

export default async function MachineBrainPage({ params }: {
  params: Promise<{ machineId: string }>;
}) {
  // Synthetic memberships are a development preview until FIN-89 supplies real reads.
  if (process.env.NODE_ENV !== "development") notFound();
  const { machineId } = await params;
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access) redirect("/dashboard");
  if (access.machineId !== machineId) {
    redirect(`/dashboard/machines/${encodeURIComponent(access.machineId)}/brain`);
  }
  return (
    <section className={`brain-view ${headingStyles.page}`} aria-label="Brain">
      <BrainOverviewTable key={access.machineId} agentName={access.displayName} machineId={access.machineId} />
    </section>
  );
}
