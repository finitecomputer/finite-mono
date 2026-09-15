"use client";

// FIN-89: compare cards, rows, and category columns in the real dashboard shell.
// Synthetic memberships only. This prototype proves no Brain access contract.
import { useEffect, useRef, useState } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { ArrowLeftIcon, ArrowRightIcon, BrainIcon, Building2Icon, Clock3Icon, FolderIcon, RefreshCwIcon, UserRoundIcon } from "lucide-react";

import { Button } from "@/components/ui/button";

type Brain = { id: string; name: string; kind: "Personal" | "Organization"; role: string; folders: string[] };
type Scenario = "ready" | "empty" | "unavailable" | "stale" | "unauthorized";
type Scope = "agent" | "human";
const VARIANTS = ["cards", "rows", "columns"] as const;
type Variant = typeof VARIANTS[number];
const LABELS = { cards: "A · Categorized cards", rows: "B · Compact list", columns: "C · Category columns" };
const SAMPLES: Record<Scope, Brain[]> = {
  agent: [
    { id: "sample-personal", name: "Personal knowledge", kind: "Personal", role: "Personal Agent", folders: ["Notes", "Projects", "Reading"] },
    { id: "sample-team", name: "Studio handbook", kind: "Organization", role: "Member", folders: ["Engineering", "Operations", "Research", "Runbooks", "Support", "Team"] },
    { id: "sample-research", name: "Research library", kind: "Organization", role: "Guest", folders: ["Shared research"] },
  ],
  human: [
    { id: "sample-personal", name: "Personal knowledge", kind: "Personal", role: "Owner", folders: ["Notes", "Projects", "Reading"] },
    { id: "sample-team", name: "Studio handbook", kind: "Organization", role: "Admin", folders: ["Engineering", "Finance", "Operations", "People", "Research", "Runbooks", "Support", "Team"] },
  ],
};
const controlClass = "rounded-md border border-border bg-background px-2 py-1.5 text-sm";

export function BrainOverviewPrototype() {
  const query = useSearchParams();
  const pathname = usePathname();
  const router = useRouter();
  const requested = query.get("variant");
  const variant = VARIANTS.find((value) => value === requested) ?? "cards";
  const [scenario, setScenario] = useState<Scenario>("ready");
  const [scope, setScope] = useState<Scope>("agent");

  useEffect(() => {
    function keydown(event: KeyboardEvent) {
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      if (event.target instanceof Element && event.target.closest("input, textarea, select, button, [contenteditable]")) return;
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
      event.preventDefault();
      const offset = event.key === "ArrowRight" ? 1 : -1;
      const next = VARIANTS[(VARIANTS.indexOf(variant) + offset + VARIANTS.length) % VARIANTS.length];
      const params = new URLSearchParams(query.toString());
      params.set("variant", next);
      router.replace(`${pathname}?${params}`, { scroll: false });
    }
    window.addEventListener("keydown", keydown);
    return () => window.removeEventListener("keydown", keydown);
  }, [pathname, query, router, variant]);

  function cycle(offset: number) {
    const params = new URLSearchParams(query.toString());
    params.set("variant", VARIANTS[(VARIANTS.indexOf(variant) + offset + VARIANTS.length) % VARIANTS.length]);
    router.replace(`${pathname}?${params}`, { scroll: false });
  }

  return (
    <div className="ocean-page-stack pb-24">
      <aside className="flex flex-wrap items-center gap-x-5 gap-y-3 rounded-xl border border-dashed border-border p-4 text-sm">
        <div className="mr-auto"><strong>Local prototype</strong><p className="text-muted-foreground">Sample data · layout and states only</p></div>
        <div className="flex items-center gap-2"><label htmlFor="prototype-scope">View as</label>
          <select id="prototype-scope" className={controlClass} value={scope} onChange={(event) => setScope(event.target.value as Scope)}>
            <option value="agent">Moss (example agent)</option><option value="human">You (example account)</option>
          </select>
        </div>
        <div className="flex items-center gap-2"><label htmlFor="prototype-scenario">Preview state</label>
          <select id="prototype-scenario" className={controlClass} value={scenario} onChange={(event) => setScenario(event.target.value as Scenario)}>
            <option value="ready">Available</option><option value="empty">No memberships</option>
            <option value="unavailable">Service unavailable</option><option value="stale">Refresh failed</option>
            <option value="unauthorized">Access lost</option>
          </select>
        </div>
      </aside>

      <MembershipPreview key={`${scope}:${scenario}`} scope={scope} scenario={scenario} variant={variant} />

      {process.env.NODE_ENV === "development" && (
        <nav aria-label="Prototype layouts" className="fixed bottom-5 left-1/2 z-50 flex max-w-[calc(100vw-2rem)] -translate-x-1/2 items-center gap-2 rounded-full bg-foreground p-2 text-background shadow-xl">
          <button aria-label="Previous layout" className="rounded-full p-2 hover:opacity-70 focus-visible:outline-2" onClick={() => cycle(-1)}><ArrowLeftIcon className="size-4" /></button>
          <span className="min-w-40 text-center text-xs font-medium sm:min-w-48">{LABELS[variant]}</span>
          <button aria-label="Next layout" className="rounded-full p-2 hover:opacity-70 focus-visible:outline-2" onClick={() => cycle(1)}><ArrowRightIcon className="size-4" /></button>
        </nav>
      )}
    </div>
  );
}

function MembershipPreview({ scope, scenario, variant }: { scope: Scope; scenario: Scenario; variant: Variant }) {
  const [brains, setBrains] = useState<Brain[] | null>(null);
  const [refreshedAt, setRefreshedAt] = useState<string | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);
  const identity = scope === "agent" ? "Moss" : "you";

  useEffect(() => {
    pending.current = setTimeout(() => {
      if (scenario === "unauthorized") {
        setError("Access is no longer available. Sign in again to refresh your Brain memberships.");
      } else if (scenario === "unavailable") {
        setError("Brain is unavailable right now. Try refreshing in a moment.");
      } else {
        setBrains(scenario === "empty" ? [] : SAMPLES[scope]);
        setRefreshedAt(new Date(Date.now() - (scenario === "stale" ? 300_000 : 0)).toISOString());
        if (scenario === "stale") setError("Refresh failed. Showing the last successful result.");
      }
      setBusy(false);
    }, 450);
    return () => { if (pending.current) clearTimeout(pending.current); };
  }, [scenario, scope]);

  function refresh() {
    setBusy(true);
    pending.current = setTimeout(() => {
      if (scenario === "ready" || scenario === "empty") {
        setBrains(scenario === "empty" ? [] : SAMPLES[scope]);
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
          <div><h1 className="ocean-page-hero__title">Brain</h1><p className="ocean-page-hero__description">Brain memberships for {identity}.</p></div>
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
        {brains?.length === 0 ? <div className="ocean-empty-state"><BrainIcon className="mx-auto mb-3 size-6" /><h2 className="font-semibold text-foreground">No Brain memberships</h2><p className="mt-1">Brains shared with {identity} will appear here after you refresh.</p></div> : null}
        {ordered.length > 0 && variant === "cards" && <VariantCards brains={ordered} />}
        {ordered.length > 0 && variant === "rows" && <VariantRows brains={ordered} />}
        {ordered.length > 0 && variant === "columns" && <VariantColumns brains={ordered} />}
      </section>

      {invitations.length > 0 && <section className="rounded-xl border border-dashed border-border p-4">
        <h2 className="text-sm font-semibold">Pending invitations <span className="ml-1 text-muted-foreground">{invitations.length}</span></h2>
        <p className="mt-1 text-sm text-muted-foreground">These are not included in the membership count.</p>
        {invitations.map((invite) => <div key={invite.name} className="mt-4 flex flex-wrap items-center justify-between gap-2 text-sm"><span>{invite.name} <span className="ml-2 text-muted-foreground">{invite.kind}</span></span><span className="ocean-chip ocean-chip--muted">Invited</span></div>)}
      </section>}

      <details className="text-xs text-muted-foreground">
        <summary className="cursor-pointer">Prototype state · synthetic data</summary>
        <pre className="mt-3 overflow-auto rounded-lg bg-muted p-3">{JSON.stringify({ scope, scenario, layout: variant, busy, refreshedAt, error, brains, invitations }, null, 2)}</pre>
      </details>
    </>
  );
}

function CategoryIcon({ kind }: { kind: Brain["kind"] }) {
  const Icon = kind === "Personal" ? UserRoundIcon : Building2Icon;
  return <Icon className="size-4 text-muted-foreground" />;
}

function VariantCards({ brains }: { brains: Brain[] }) {
  return <div className="space-y-7">{(["Personal", "Organization"] as const).map((kind) => {
    const group = brains.filter((brain) => brain.kind === kind);
    if (!group.length) return null;
    return <section key={kind}><h2 className="mb-3 flex items-center gap-2 text-sm font-semibold"><CategoryIcon kind={kind} />{kind}<span className="font-normal text-muted-foreground">{group.length}</span></h2>
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">{group.map((brain) => <article key={brain.id} className="ocean-skill-card">
        <div className="ocean-skill-card__meta"><span className="ocean-chip">{brain.kind}</span></div>
        <div className="ocean-skill-card__copy"><h3>{brain.name}</h3></div>
        <div className="mt-3 min-w-0 text-sm">
          <p className="flex items-center gap-1.5 text-muted-foreground" title="Folders listed for this identity. Linked folders and local sync status are not included.">
            <FolderIcon className="size-3.5" aria-hidden />
            {brain.folders.length} {brain.folders.length === 1 ? "folder" : "folders"} shown
          </p>
          {brain.folders.length > 0 && <p className="mt-1.5 break-words leading-relaxed text-muted-foreground">
            {brain.folders.slice(0, 3).join(" · ")}
            {brain.folders.length > 3 && <span aria-label={`${brain.folders.length - 3} more folders`}> · +{brain.folders.length - 3}</span>}
          </p>}
        </div>
        <div className="mt-4 flex items-center justify-between gap-2 border-t border-border pt-3 text-sm"><span className="text-muted-foreground">Role</span><span>{brain.role}</span></div>
      </article>)}</div>
    </section>;
  })}</div>;
}

function VariantRows({ brains }: { brains: Brain[] }) {
  return <div className="overflow-hidden rounded-xl border border-border">{(["Personal", "Organization"] as const).map((kind) => {
    const group = brains.filter((brain) => brain.kind === kind);
    if (!group.length) return null;
    return <section key={kind}><h2 className="flex items-center gap-2 bg-muted/40 px-4 py-3 text-sm font-semibold"><CategoryIcon kind={kind} />{kind}<span className="text-muted-foreground">{group.length}</span></h2>
      {group.map((brain) => <article key={brain.id} className="flex flex-wrap items-center justify-between gap-3 border-t border-border bg-card px-4 py-5"><h3 className="text-sm font-medium">{brain.name}</h3><span className="ocean-chip">{brain.role}</span></article>)}
    </section>;
  })}</div>;
}

function VariantColumns({ brains }: { brains: Brain[] }) {
  return <div className="grid gap-4 lg:grid-cols-2">{(["Personal", "Organization"] as const).map((kind) => {
    const group = brains.filter((brain) => brain.kind === kind);
    if (!group.length) return null;
    return <section key={kind} className="rounded-xl border border-border bg-card p-5"><div className="mb-5 flex items-start justify-between"><div><CategoryIcon kind={kind} /><h2 className="mt-3 text-lg font-semibold">{kind}</h2></div><span className="text-3xl font-semibold text-muted-foreground">{group.length}</span></div>
      <div className="divide-y divide-border">{group.map((brain) => <article key={brain.id} className="py-4"><h3 className="text-sm font-medium">{brain.name}</h3><p className="mt-1 text-sm text-muted-foreground">{brain.role}</p></article>)}</div>
    </section>;
  })}</div>;
}
