"use client";
import { RefreshCwIcon } from "lucide-react";

export function AgentInventoryRefresh({ busy, loadedAt, error, refresh, label }: {
  busy: boolean; loadedAt: string | null; error: string | null; refresh: () => Promise<void>; label: string;
}) {
  return <div className="flex flex-wrap items-center gap-2 py-3 text-sm text-muted-foreground">
    <button className="inline-flex items-center gap-2 rounded-md border px-3 py-2 disabled:opacity-50" type="button" disabled={busy} onClick={() => void refresh()} aria-label={`Refresh ${label}`}>
      <RefreshCwIcon size={16} aria-hidden className={busy ? "animate-spin" : ""} />{busy ? "Refreshing…" : "Refresh"}
    </button>
    <span aria-live="polite">{loadedAt ? <>Updated <time dateTime={loadedAt}>{new Date(loadedAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}</time></> : busy ? "Loading…" : "No successful refresh yet"}</span>
    {error && <p role="alert" className="w-full text-amber-700 dark:text-amber-400">{error}{loadedAt ? " Showing the last successful list." : ""}</p>}
  </div>;
}
