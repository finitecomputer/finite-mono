import Link from "next/link";
import { redirect } from "next/navigation";
import { BrainIcon, MessageSquareIcon, PlugIcon } from "lucide-react";

import { FiniteBrand } from "@/components/finite-brand";
import { Button } from "@/components/ui/button";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

export default async function MachineBrainPage({
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

  const base = `/dashboard/machines/${encodeURIComponent(access.machineId)}`;
  const enabled = Boolean(process.env.FC_BRAIN_UPSTREAM_URL?.trim());
  return (
    <div className="finite-product-surface">
      <header className="finite-product-surface__bar">
        <FiniteBrand href={`${base}/chat`} />
        <div className="finite-product-surface__identity">
          <BrainIcon className="size-4" />
          <span>Brain</span>
          <small>{account.email ?? access.displayName}</small>
        </div>
        <nav className="finite-product-surface__actions" aria-label="Agent products">
          <Button asChild variant="ghost" size="sm">
            <Link href={`${base}/connections`}><PlugIcon />Connections</Link>
          </Button>
          <Button asChild variant="outline" size="sm">
            <Link href={`${base}/chat`}><MessageSquareIcon />Chat</Link>
          </Button>
        </nav>
      </header>

      {enabled ? (
        <iframe
          className="finite-product-surface__frame"
          src="/client"
          title={`${access.displayName} Brain`}
          allow="clipboard-read; clipboard-write"
        />
      ) : (
        <main className="finite-product-surface__empty">
          <BrainIcon className="size-10" />
          <h1>Brain is not connected</h1>
          <p>Configure an internal FiniteBrain origin to serve the first-party Product Client here.</p>
        </main>
      )}
    </div>
  );
}
