"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import type { HostedGatewayStatus } from "@/lib/hosted-gateway";

export function HostedGatewayCard({ machineId }: { machineId: string }) {
  const [status, setStatus] = useState<HostedGatewayStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);

  async function update(action: "status" | "enable" | "disable") {
    setBusy(true); setError(null); setCopied(null); setRevealed(false);
    try {
      const response = await fetch(`/api/admin/machines/${encodeURIComponent(machineId)}/gateway`, {
        method: action === "status" ? "GET" : "POST", cache: "no-store",
        headers: action === "status" ? undefined : { "Content-Type": "application/json" },
        body: action === "status" ? undefined : JSON.stringify({ action }),
      });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || "Gateway is unavailable.");
      setStatus(result);
    } catch (failure) {
      setStatus(null);
      setError(failure instanceof Error ? failure.message : "Gateway is unavailable.");
    } finally { setBusy(false); }
  }

  async function copy(label: string, value: string) {
    try { await navigator.clipboard.writeText(value); setCopied(`${label} copied`); }
    catch { setError("Clipboard unavailable. Reveal the value and copy it manually."); }
  }

  return <section className="rounded-xl border bg-card p-5">
    <h2 className="font-semibold">Hosted gateway <span className="ml-2 text-xs font-normal text-muted-foreground">Team preview</span></h2>
    <p className="mt-1 max-w-2xl text-sm text-muted-foreground">Connect Hermes Desktop or your local chat UI to this agent.</p>
    <div className="mt-4 flex flex-wrap items-center gap-3">
      {status ? <>
        <button type="button" role="switch" aria-checked={status.enabled} aria-label="Enable hosted gateway" disabled={busy}
          onClick={() => void update(status.enabled ? "disable" : "enable")}
          className={`relative h-6 w-11 rounded-full transition-colors disabled:opacity-50 ${status.enabled ? "bg-primary" : "bg-muted-foreground/40"}`}>
          <span className={`absolute top-0.5 size-5 rounded-full bg-background transition-transform ${status.enabled ? "left-0.5 translate-x-5" : "left-0.5"}`} />
        </button>
        <span className="text-sm">{busy ? "Updating…" : status.ready ? "Enabled" : status.enabled ? "Starting…" : "Disabled"}</span>
        {status.enabled && !status.ready ? <Button variant="outline" disabled={busy} onClick={() => void update("status")}>Check connection</Button> : null}
      </> : <Button variant="outline" disabled={busy} onClick={() => void update("status")}>{busy ? "Loading…" : "Load gateway controls"}</Button>}
    </div>
    {status?.enabled ? <p className="mt-3 text-xs text-muted-foreground">Disabling disconnects gateway clients and revokes this credential.</p> : null}
    {status?.ready && status.token ? <div className="mt-4 space-y-3">
      <div><p className="text-xs font-medium">WebSocket URL</p><code className="mt-1 block break-all text-sm">{status.url}</code>
        <Button size="sm" variant="outline" className="mt-2" onClick={() => void copy("URL", status.url)}>Copy URL</Button></div>
      <div><label htmlFor="gateway-token" className="text-xs font-medium">Session token</label>
        <input id="gateway-token" readOnly autoComplete="off" type={revealed ? "text" : "password"} value={status.token} className="mt-1 block w-full rounded border bg-background p-2 font-mono text-sm" />
        <div className="mt-2 flex gap-2"><Button size="sm" variant="outline" onClick={() => setRevealed(!revealed)}>{revealed ? "Hide token" : "Reveal token"}</Button>
          <Button size="sm" variant="outline" onClick={() => void copy("Token", status.token!)}>Copy token</Button></div>
      </div>
    </div> : null}
    <p role="status" className="mt-2 text-xs text-muted-foreground">{copied}</p>
    {error ? <p role="alert" className="mt-3 text-sm text-destructive">{error}</p> : null}
  </section>;
}
