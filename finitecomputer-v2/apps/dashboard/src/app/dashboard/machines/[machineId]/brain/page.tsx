import { redirect } from "next/navigation";
import { BrainIcon } from "lucide-react";

import { BrainFrame, BrainHeader } from "@/components/brain-frame";
import { fetchRuntimeAgentNpub } from "@/lib/agent-contact";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

export default async function MachineBrainPage({
  params,
}: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access) redirect("/dashboard");
  if (access.machineId !== machineId) {
    redirect(`/dashboard/machines/${encodeURIComponent(access.machineId)}/brain`);
  }

  const enabled = Boolean(process.env.FC_BRAIN_UPSTREAM_URL?.trim());
  const agentNpub = enabled ? await fetchRuntimeAgentNpub(access.primaryUrl) : null;
  return (
    <div className="finite-brain-page">
      <BrainHeader />
      <div className="finite-brain-page__body">
        {enabled ? (
          <BrainFrame title={`${access.displayName} Brain`} agentNpub={agentNpub} />
        ) : (
          <main className="finite-product-surface__empty">
            <BrainIcon className="size-10" />
            <h2>Brain isn&apos;t available right now</h2>
            <p>Try again in a few minutes.</p>
          </main>
        )}
      </div>
    </div>
  );
}
