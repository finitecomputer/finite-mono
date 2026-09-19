"use client";
import { BrainChatState } from "@/components/brain-chat-state";
import { BrainTable } from "@/components/brain-membership-table";
import { AgentInventoryRefresh } from "@/components/agent-inventory-refresh";
import { useAgentInventory } from "@/hooks/use-agent-inventory";
import { readBrainInventory } from "@/lib/agent-product-inventory";
import headingStyles from "@/styles/agent-page-heading.module.css";

export function AgentBrainBrowser({ runtimeId, agentName }: { runtimeId: string; agentName: string }) {
  const inventory = useAgentInventory(runtimeId, readBrainInventory);
  const brains = inventory.data?.brains;
  const accessible = brains?.filter(brain => !brain.pending).length ?? 0;
  const pending = brains?.filter(brain => brain.pending).length ?? 0;
  return <>
    <header className="brain-heading">
      <h1 className={headingStyles.title}>Brain</h1>
      <p className={headingStyles.subtitle} aria-live="polite">{brains
        ? `${accessible} ${accessible === 1 ? "brain" : "brains"} accessible to ${agentName}.${pending ? ` ${pending} pending.` : ""}`
        : inventory.busy ? `Loading brains accessible to ${agentName}…` : `Brain access unavailable for ${agentName}`}</p>
    </header>
    <section aria-label="Brain memberships" aria-busy={inventory.busy}>
      {brains && brains.length > 0 && <BrainTable key={inventory.revision} brains={brains} />}
      {brains?.length === 0 && <BrainChatState agentName={agentName} machineId={runtimeId} />}
      {!brains && !inventory.busy && <BrainChatState agentName={agentName} machineId={runtimeId} unavailable />}
    </section>
    <AgentInventoryRefresh {...inventory} label="brains" />
    {brains && brains.length > 0 && <p className="text-xs text-muted-foreground">Shows folder information this agent can access. Linked folders aren’t included.</p>}
  </>;
}
