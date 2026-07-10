import { redirect } from "next/navigation";
import { BrainIcon } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

export default async function MachineBrainPage({
  params,
}: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access) redirect("/dashboard");

  const enabled = Boolean(process.env.FC_BRAIN_UPSTREAM_URL?.trim());
  return (
    <div className="space-y-6">
      <PageHeader title="Brain" description={`What ${access.displayName} remembers.`} />
      {enabled ? (
        <div className="h-[calc(100vh-12rem)] min-h-[36rem] overflow-hidden rounded-[var(--radius-card)] border border-border bg-card">
          <iframe
            className="size-full border-0"
            src="/client"
            title={`${access.displayName} Brain`}
            allow="clipboard-read; clipboard-write"
          />
        </div>
      ) : (
        <main className="finite-product-surface__empty rounded-[var(--radius-card)] border border-border bg-card">
          <BrainIcon className="size-10" />
          <h2>Brain isn&apos;t available right now</h2>
          <p>Try again in a few minutes.</p>
        </main>
      )}
    </div>
  );
}
