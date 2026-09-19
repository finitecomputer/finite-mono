"use client";

// Adapted from Austin’s FIN-89 prototype, PR #888 at 7b342c99.
// Compact table presentation of selected-agent metadata.
// Synthetic memberships only. This prototype proves no Brain access contract.
import { useEffect, useRef, useState } from "react";
import { BrainChatState } from "@/components/brain-chat-state";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { RefreshCwIcon } from "lucide-react";

import headingStyles from "@/styles/agent-page-heading.module.css";
import { BrainTable } from "@/components/brain-membership-table";


type Brain = {
  id: string;
  name: string;
  kind: "Personal" | "Organization";
  role: string;
  folders: string[] | null;
  pending?: boolean;
};
type Scenario =
  | "ready"
  | "empty"
  | "unavailable"
  | "stale"
  | "unauthorized"
  | "folder-details-unavailable";
const SAMPLE_AGENT_BRAINS: Brain[] = [
  { id: "personal", name: "Personal brain", kind: "Personal", role: "Personal agent", folders: ["Life", "Learning"] },
  { id: "finite", name: "Finite", kind: "Organization", role: "Member", folders: ["Company", "Product", "Research", "Projects", "Operations", "Runbooks"] },
  { id: "design-studio", name: "Design studio", kind: "Organization", role: "Guest", folders: ["Studio", "Client work"] },
];
const controlClass = "rounded-md border border-border bg-background px-2 py-1.5 text-sm";

export function BrainOverviewTable({ agentName, machineId }: { agentName: string; machineId: string }) {
  const [scenario, setScenario] = useState<Scenario>("ready");

  return (
    <div className="brain-table-preview">
      <p className="text-sm text-muted-foreground">Design preview · Sample Brain memberships</p>
      <MembershipPreview key={scenario} scenario={scenario} agentName={agentName} machineId={machineId} />
      <details className="brain-preview-options">
        <summary>Preview states</summary>
        <label htmlFor="prototype-scenario">Sample state</label>{" "}
        <select id="prototype-scenario" className={controlClass} value={scenario} onChange={(event) => setScenario(event.target.value as Scenario)}>
          <option value="ready">Available</option><option value="empty">No memberships</option>
          <option value="unavailable">Service unavailable</option><option value="stale">Refresh failed</option>
          <option value="unauthorized">Access lost</option><option value="folder-details-unavailable">Folder details unavailable</option>
        </select>
      </details>
    </div>
  );
}

function sampleMemberships(scenario: Scenario): Brain[] {
  if (scenario === "empty") return [];
  return SAMPLE_AGENT_BRAINS.map((brain) => (
    scenario === "folder-details-unavailable" && brain.id === "finite"
      ? { ...brain, folders: null }
      : brain
  ));
}

function MembershipPreview({ scenario, agentName, machineId }: { scenario: Scenario; agentName: string; machineId: string }) {
  const [brains, setBrains] = useState<Brain[] | null>(null);
  const [refreshedAt, setRefreshedAt] = useState<string | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    pending.current = setTimeout(() => {
      if (scenario === "unauthorized") {
        setError("Access to this agent’s Brain overview is no longer available.");
      } else if (scenario === "unavailable") {
        setError("Brain is unavailable right now. Try refreshing in a moment.");
      } else {
        setBrains(sampleMemberships(scenario));
        setRefreshedAt(new Date(Date.now() - (scenario === "stale" ? 300_000 : 0)).toISOString());
        if (scenario === "stale") setError("Refresh failed. Showing the last successful result.");
      }
      setBusy(false);
    }, 450);
    return () => { if (pending.current) clearTimeout(pending.current); };
  }, [scenario]);

  function refresh() {
    setBusy(true);
    pending.current = setTimeout(() => {
      if (scenario === "ready" || scenario === "empty" || scenario === "folder-details-unavailable") {
        setBrains(sampleMemberships(scenario));
        setRefreshedAt(new Date().toISOString());
        setError(null);
      }
      setBusy(false);
    }, 650);
  }

  const invitations: Brain[] = brains && brains.length > 0
    ? [{ id: "community-notes-invitation", name: "Community notes", kind: "Organization", role: "Pending", folders: null, pending: true }]
    : [];
  const ordered = [...(brains ?? []), ...invitations].sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
  const initialLoading = brains === null && busy;

  return (
    <>
      <header className="brain-heading">
        <h1 className={headingStyles.title}>Brain</h1>
        <p className={headingStyles.subtitle} aria-live="polite"><span className={initialLoading ? "brain-loading-text" : undefined}>{brains === null
            ? (busy ? `Loading brains accessible to ${agentName}…` : `Brain access unavailable for ${agentName}`)
            : `${brains.length} ${brains.length === 1 ? "brain" : "brains"} accessible to ${agentName}.${invitations.length > 0 ? ` ${invitations.length} ${invitations.length === 1 ? "brain" : "brains"} pending.` : ""}`}</span></p>
      </header>

      {error && !refreshedAt && <div role="alert" className="rounded-xl border border-amber-500/30 bg-amber-500/5 px-4 py-3 text-sm">{error}</div>}

      {!initialLoading && <>
      <section aria-label="Brain memberships" aria-busy={busy}>
        {brains?.length === 0 ? <BrainChatState agentName={agentName} machineId={machineId} /> : null}
        {ordered.length > 0 && <BrainTable brains={ordered.map(brain => ({ ...brain, pending: Boolean(brain.pending), folders: brain.folders?.map((name, index) => ({ id: String(index), name })) ?? null }))} />}
      </section>

      <div className="brain-table-refresh">
        <Tooltip>
          <TooltipTrigger asChild>
            <button type="button" aria-label={busy ? "Refreshing brains" : "Refresh brains"} onClick={refresh} disabled={busy || scenario === "unauthorized"}>
              <RefreshCwIcon size={16} aria-hidden="true" className={busy ? "animate-spin" : ""} />
            </button>
          </TooltipTrigger>
          <TooltipContent sideOffset={0}>Refresh brains</TooltipContent>
        </Tooltip>
        <span aria-live="polite">{refreshedAt ? <>Updated <time dateTime={refreshedAt}>{new Date(refreshedAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}</time></> : "No successful refresh yet"}</span>
        {error && refreshedAt && <span role="alert" className="brain-refresh-error">{error}</span>}
      </div>
      </>}
    </>
  );
}
