"use client";

// FIN-89: the selected categorized-card design in the real dashboard shell.
// Synthetic memberships only. This prototype proves no Brain access contract.
import { useEffect, useRef, useState } from "react";
import { BrainIcon, Building2Icon, Clock3Icon, FolderIcon, RefreshCwIcon, UserRoundIcon } from "lucide-react";

import { Button } from "@/components/ui/button";

type Brain = {
  id: string;
  name: string;
  kind: "Personal" | "Organization";
  role: string;
  folders: string[] | null;
};
type Scenario =
  | "ready"
  | "empty"
  | "unavailable"
  | "stale"
  | "unauthorized"
  | "folder-details-unavailable";
const SAMPLE_AGENT_BRAINS: Brain[] = [
    { id: "sample-personal", name: "Personal knowledge", kind: "Personal", role: "Personal Agent", folders: ["Notes", "Projects", "Reading"] },
    { id: "sample-team", name: "Studio handbook", kind: "Organization", role: "Member", folders: ["Engineering", "Operations", "Research", "Runbooks", "Support", "Team"] },
    { id: "sample-research", name: "Research library", kind: "Organization", role: "Guest", folders: ["Shared research"] },
];
const controlClass = "rounded-md border border-border bg-background px-2 py-1.5 text-sm";

export function BrainOverviewPrototype() {
  const [scenario, setScenario] = useState<Scenario>("ready");

  return (
    <div className="ocean-page-stack">
      <aside className="flex flex-wrap items-center gap-x-5 gap-y-3 rounded-xl border border-dashed border-border p-4 text-sm">
        <div className="mr-auto"><strong>Local prototype</strong><p className="text-muted-foreground">Sample data · layout and states only</p></div>
        <div className="flex items-center gap-2"><label htmlFor="prototype-scenario">Preview state</label>
          <select id="prototype-scenario" className={controlClass} value={scenario} onChange={(event) => setScenario(event.target.value as Scenario)}>
            <option value="ready">Available</option><option value="empty">No memberships</option>
            <option value="unavailable">Service unavailable</option><option value="stale">Refresh failed</option>
            <option value="unauthorized">Access lost</option>
            <option value="folder-details-unavailable">Folder details unavailable</option>
          </select>
        </div>
      </aside>

      <MembershipPreview key={scenario} scenario={scenario} />
    </div>
  );
}

function sampleMemberships(scenario: Scenario): Brain[] {
  if (scenario === "empty") return [];
  return SAMPLE_AGENT_BRAINS.map((brain) => (
    scenario === "folder-details-unavailable" && brain.id === "sample-team"
      ? { ...brain, folders: null }
      : brain
  ));
}

function MembershipPreview({ scenario }: { scenario: Scenario }) {
  const [brains, setBrains] = useState<Brain[] | null>(null);
  const [refreshedAt, setRefreshedAt] = useState<string | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);
  const agentName = "Moss";

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

  const ordered = [...(brains ?? [])].sort((a, b) => a.name.localeCompare(b.name));
  const invitations = brains && brains.length > 0 ? [{ name: "Community notes", kind: "Organization" }] : [];

  return (
    <>
      <section className="ocean-page-hero">
        <div className="ocean-page-hero__main">
          <span className="ocean-page-hero__icon"><BrainIcon className="size-5" /></span>
          <div><h1 className="ocean-page-hero__title">Brain</h1><p className="ocean-page-hero__description">Brain memberships for {agentName}.</p></div>
        </div>
        <div className="flex flex-wrap items-center justify-start gap-5 sm:justify-end">
          <div className="mr-auto sm:mr-0"><strong className="text-2xl font-semibold">{brains?.length ?? "—"}</strong><span className="ml-2 text-sm text-muted-foreground">{brains?.length === 1 ? "Brain" : "Brains"}</span></div>
          <Button variant="outline" onClick={refresh} disabled={busy || scenario === "unauthorized"}>
            <RefreshCwIcon className={busy ? "animate-spin" : ""} />{busy ? "Refreshing…" : "Refresh"}
          </Button>
        </div>
      </section>

      <div className="flex flex-wrap items-center justify-between gap-2 text-sm text-muted-foreground" aria-live="polite">
        <span>Memberships describe access, not local sync status.</span>
        <span className="flex items-center gap-1.5"><Clock3Icon className="size-3.5" />{refreshedAt ? <>Last refreshed <time dateTime={refreshedAt}>{new Date(refreshedAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit", second: "2-digit" })}</time>{error ? " · Stale" : ""}</> : "No successful refresh yet"}</span>
      </div>

      {error && <div role="alert" className="rounded-xl border border-amber-500/30 bg-amber-500/5 px-4 py-3 text-sm">{error}</div>}

      <section aria-label="Brain memberships" aria-busy={busy}>
        {brains === null && busy ? <div role="status" className="ocean-empty-state">Loading Brain memberships…</div> : null}
        {brains?.length === 0 ? <div className="ocean-empty-state"><BrainIcon className="mx-auto mb-3 size-6" /><h2 className="font-semibold text-foreground">No Brain memberships</h2><p className="mt-1">Brains shared with {agentName} will appear here after you refresh.</p></div> : null}
        {ordered.length > 0 && <BrainCards brains={ordered} />}
      </section>

      {invitations.length > 0 && <section className="rounded-xl border border-dashed border-border p-4">
        <h2 className="text-sm font-semibold">Pending invitations <span className="ml-1 text-muted-foreground">{invitations.length}</span></h2>
        <p className="mt-1 text-sm text-muted-foreground">These are not included in the membership count.</p>
        {invitations.map((invite) => <div key={invite.name} className="mt-4 flex flex-wrap items-center justify-between gap-2 text-sm"><span>{invite.name} <span className="ml-2 text-muted-foreground">{invite.kind}</span></span><span className="ocean-chip ocean-chip--muted">Invited</span></div>)}
      </section>}

      <details className="text-xs text-muted-foreground">
        <summary className="cursor-pointer">Prototype state · synthetic data</summary>
        <pre className="mt-3 overflow-auto rounded-lg bg-muted p-3">{JSON.stringify({ agentName, scenario, busy, refreshedAt, error, brains, invitations }, null, 2)}</pre>
      </details>
    </>
  );
}

function CategoryIcon({ kind }: { kind: Brain["kind"] }) {
  const Icon = kind === "Personal" ? UserRoundIcon : Building2Icon;
  return <Icon className="size-4 text-muted-foreground" />;
}

function FolderNames({ brainId, folders }: { brainId: string; folders: string[] }) {
  const [expanded, setExpanded] = useState(false);
  const remaining = folders.length - 3;
  return (
    <p className="mt-1.5 leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">
      <span id={`${brainId}-folders`}>
        {(expanded ? folders : folders.slice(0, 3)).join(" · ")}
      </span>
      {remaining > 0 && (
        <>
          {" · "}
          <button
            type="button"
            aria-expanded={expanded}
            aria-controls={`${brainId}-folders`}
            aria-label={expanded ? "Show fewer folders" : `Show ${remaining} more folders`}
            className="rounded-sm font-medium text-foreground underline decoration-current/40 underline-offset-4 hover:decoration-current focus-visible:outline-2 focus-visible:outline-offset-2"
            onClick={() => setExpanded((value) => !value)}
          >
            {expanded ? "Show less" : `+${remaining} more`}
          </button>
        </>
      )}
    </p>
  );
}

function BrainCards({ brains }: { brains: Brain[] }) {
  return (
    <div className="space-y-7">
      {(["Personal", "Organization"] as const).map((kind) => {
        const group = brains.filter((brain) => brain.kind === kind);
        if (!group.length) return null;
        return (
          <section key={kind}>
            <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
              <CategoryIcon kind={kind} />
              {kind}
              <span className="font-normal text-muted-foreground">{group.length}</span>
            </h2>
            <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
              {group.map((brain) => (
                <article
                  key={brain.id}
                  aria-labelledby={`${brain.id}-name`}
                  className="ocean-skill-card min-w-0"
                  style={{ gridTemplateRows: "auto auto 1fr auto" }}
                >
                  <div className="ocean-skill-card__meta">
                    <span className="ocean-chip">{brain.kind}</span>
                  </div>
                  <div className="ocean-skill-card__copy">
                    <h3 id={`${brain.id}-name`} className="[overflow-wrap:anywhere]">{brain.name}</h3>
                  </div>
                  {brain.folders === null ? (
                    <p className="mt-3 text-sm text-muted-foreground">Folder details unavailable</p>
                  ) : (
                    <div className="mt-3 min-w-0 text-sm">
                      <p
                        className="flex items-center gap-1.5 text-muted-foreground"
                        title="Folders listed for this agent. Linked folders and local sync status are not included."
                      >
                        <FolderIcon className="size-3.5" aria-hidden />
                        {brain.folders.length} {brain.folders.length === 1 ? "folder" : "folders"} shown
                      </p>
                      {brain.folders.length > 0 && (
                        <FolderNames brainId={brain.id} folders={brain.folders} />
                      )}
                    </div>
                  )}
                  <dl className="mt-4 flex items-center justify-between gap-2 border-t border-border pt-3 text-sm">
                    <dt className="text-muted-foreground">Role</dt>
                    <dd>{brain.role}</dd>
                  </dl>
                </article>
              ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}
