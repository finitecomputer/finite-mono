"use client";

// Adapted from Austin’s FIN-89 prototype, PR #888 at 7b342c99.
// Compact table presentation of selected-agent metadata.
// Synthetic memberships only. This prototype proves no Brain access contract.
import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { useHostedChat } from "@/components/hosted-chat-provider";
import { canonicalNewChatTopic } from "@/lib/hosted-web-chat-topics";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { BrainIcon, ChevronDownIcon, RefreshCwIcon } from "lucide-react";

import headingStyles from "@/styles/agent-page-heading.module.css";
import tableStyles from "@/styles/dashboard-table.module.css";


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
        {brains?.length === 0 ? <BrainEmptyState agentName={agentName} machineId={machineId} /> : null}
        {ordered.length > 0 && <BrainTable brains={ordered} />}
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

function BrainTable({ brains }: { brains: Brain[] }) {
  return (
    <div className={`brain-table-scroll ${tableStyles.surface}`} role="region" aria-label="Agent brains and invitations" tabIndex={0}>
      <table className="brain-table">
        <caption className="sr-only">Brains accessible to the selected agent and pending invitations</caption>
        <thead><tr><th scope="col">Brain</th><th scope="col">Type</th><th scope="col">Access</th><th scope="col">Folders shown</th></tr></thead>
        <tbody>{brains.map((brain) => <tr key={brain.id}>
          <th scope="row">{brain.name}</th>
          <td>{brain.kind}</td>
          <td>{brain.role}</td>
          <td>{brain.pending ? null : brain.folders === null ? <span>Folder details unavailable</span> : brain.folders.length === 0 ? "0" :
            <details className="brain-table-folders"><summary aria-label={`${brain.folders.length} folders in ${brain.name}`}>{brain.folders.length}<ChevronDownIcon className="size-3.5" aria-hidden /></summary>
              <ul>{brain.folders.map((folder, index) => <li key={`${index}-${folder}`}>{folder}</li>)}</ul>
            </details>}
          </td>
        </tr>)}</tbody>
      </table>
    </div>
  );
}

function BrainEmptyState({ agentName, machineId }: { agentName: string; machineId: string }) {
  const { state, dispatch } = useHostedChat();
  const router = useRouter();
  const pending = useRef(false);
  const intentKey = useRef<string | null>(null);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function openChat() {
    if (pending.current) return;
    const roomId = state?.hosted_agent_binding?.canonical_room_id;
    const topic = canonicalNewChatTopic((state?.topics ?? []).filter((item) => item.room_id === roomId && !item.archived));
    if (!roomId || !topic) {
      setError("Chat is still connecting. Please try again in a moment.");
      return;
    }
    pending.current = true;
    setOpening(true);
    setError(null);
    intentKey.current ??= crypto.randomUUID();
    try {
      await dispatch({ StartTopicChatIntent: { room_id: roomId, topic_id: topic.topic_id, reason: null, intent_key: intentKey.current } });
      const prompt = "Help me create a brain. Start by asking whether I want a personal brain or an organization brain.";
      router.push(`/dashboard/machines/${encodeURIComponent(machineId)}/chat?${new URLSearchParams({ prompt })}`);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Couldn’t open a new chat. Please try again.");
      pending.current = false;
      setOpening(false);
    }
  }

  return (
    <div className="brain-empty-state">
      <BrainIcon aria-hidden="true" />
      <h2>Your brains will show up here</h2>
      <p>Talk to {agentName} to create your personal brain or an organization brain for your team.</p>
      <button type="button" className="brain-empty-chat" disabled={opening} onClick={() => void openChat()}>{opening ? "Opening chat…" : "Open chat"}</button>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
