import { redirect } from "next/navigation";

import { BrowserChatDevicesPanel } from "@/components/browser-chat-devices-panel";
import { wasmSpikeEnabled, WASM_SPIKE_MACHINE } from "@/lib/wasm-spike";
import { ConnectionsPanel } from "@/components/connections-panel";
import { PageHeader } from "@/components/page-header";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import { googleWorkspaceOAuthConfigured } from "@/lib/google-workspace-oauth";

export default async function MachineConnectionsPage({
  params,
}: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  if (wasmSpikeEnabled() && machineId === WASM_SPIKE_MACHINE) {
    return (
      <div className="space-y-6">
        <PageHeader title="Connections" description="Devices connected to your chat with Hermes." />
        <BrowserChatDevicesPanel />
      </div>
    );
  }
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access) redirect("/dashboard");
  if (access.machineId !== machineId) {
    redirect(
      `/dashboard/machines/${encodeURIComponent(access.machineId)}/connections`
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader title="Connections" description={`Choose how ${access.displayName} works with you.`} />
      <ConnectionsPanel
        machineId={access.machineId}
        googleConfigured={googleWorkspaceOAuthConfigured()}
      />
    </div>
  );
}
