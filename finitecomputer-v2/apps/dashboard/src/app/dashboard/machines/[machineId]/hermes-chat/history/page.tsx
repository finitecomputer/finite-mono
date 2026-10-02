import { notFound, redirect } from "next/navigation";

import { HermesHistoryCoverage } from "@/components/hermes-history-coverage";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import headingStyles from "@/styles/agent-page-heading.module.css";

export const dynamic = "force-dynamic";

/** Admin spot-check of which Finite Chat chats Hermes already holds. Read-only. */
export default async function HermesHistoryCoveragePage({
  params,
}: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  const access = await loadDashboardMachineAccess(machineId);
  if (!access) redirect("/dashboard");
  if (!access.viewer?.isAdmin) notFound();
  if (access.machineId !== machineId) {
    redirect(`/dashboard/machines/${encodeURIComponent(access.machineId)}/hermes-chat/history`);
  }
  return (
    <section className={headingStyles.page} aria-label="Hermes history coverage">
      <h1>{access.displayName}: Finite Chat history in Hermes</h1>
      <HermesHistoryCoverage key={access.machineId} runtimeId={access.machineId} />
    </section>
  );
}
