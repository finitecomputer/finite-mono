"use client";
import { AgentSitesBrowser } from "@/components/agent-sites-browser";
import { AgentInventoryRefresh } from "@/components/agent-inventory-refresh";
import { useAgentInventory } from "@/hooks/use-agent-inventory";
import { readSitesInventory } from "@/lib/agent-product-inventory";

export function AgentSitesInventory({ runtimeId, agentName }: { runtimeId: string; agentName: string }) {
  const inventory = useAgentInventory(runtimeId, readSitesInventory);
  const sourceOnly = inventory.data?.sourceOnlyProjects ?? 0;
  return <AgentSitesBrowser key={inventory.revision} machineId={runtimeId} sites={inventory.data?.sites ?? null}
    loading={inventory.busy && !inventory.data}
    subtitle={`Sites ${agentName} can work on`}
    inventoryStatus={<>
      <AgentInventoryRefresh {...inventory} label="sites" />
      {sourceOnly > 0 && <p className="text-sm text-muted-foreground">{sourceOnly} {sourceOnly === 1 ? "repository has" : "repositories have"} no website yet.</p>}
    </>} />;
}
