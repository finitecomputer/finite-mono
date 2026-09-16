"use client";

import { useEffect, useRef, useState } from "react";
import type { AgentEndpoint } from "@/lib/core-client";
import { readAgentStatus, transportControl } from "@/lib/agent-status-client";
import { Button } from "@/components/ui/button";

export function AgentStatusPanel({ projectId }: { projectId: string }) {
  const [binding, setBinding] = useState<AgentEndpoint | null>(null);
  const [result, setResult] = useState<unknown>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const active = useRef<AbortController | null>(null);
  useEffect(() => {
    let mounted = true;
    transportControl(projectId).then((value) => { if (mounted) setBinding(value); })
      .catch(() => { if (mounted) setError("This agent has not registered hosted access yet."); });
    return () => { mounted = false; active.current?.abort(); };
  }, [projectId]);
  async function toggle() {
    if (!binding) return;
    setBusy(true); setError(""); setResult(null);
    try {
      await transportControl(projectId, "PUT", {
        generation: binding.generation, endpointId: binding.endpointId, enabled: !binding.enabled,
      });
      setBinding(await transportControl(projectId));
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not change access");
      // Refresh after a stale-generation conflict, without automatically
      // repeating the mutation against a replacement endpoint.
      setBinding(await transportControl(projectId).catch(() => null));
    }
    finally { setBusy(false); }
  }
  async function check() {
    setBusy(true); setError(""); setResult(null);
    const controller = new AbortController(); active.current = controller;
    try {
      const status = await readAgentStatus(projectId, controller.signal);
      if (!controller.signal.aborted) setResult(status);
    }
    catch (e) { if (!controller.signal.aborted) setError(e instanceof Error ? e.message : "Status unavailable"); }
    finally { if (!controller.signal.aborted) setBusy(false); active.current = null; }
  }
  return <section className="space-y-4 rounded-xl border bg-card p-5">
    <h1 className="text-xl font-semibold">Agent status</h1>
    <p className="text-sm text-muted-foreground">Internal preview. Read live status directly from this agent’s Hermes server.</p>
    <p>Hosted access: {binding ? (binding.enabled ? "Enabled" : "Disabled") : "Unavailable"}</p>
    <div className="flex gap-3">
      <Button variant="outline" disabled={!binding || busy} onClick={toggle}>
        {binding?.enabled ? "Disable hosted access" : "Enable hosted access"}
      </Button>
      <Button disabled={!binding?.enabled || busy} onClick={check}>
        {busy ? "Working…" : "Check agent status"}
      </Button>
    </div>
    {error ? <p role="alert">{error}</p> : null}
    {result !== null ? <>
      <p role="status">Live Hermes status received</p>
      <pre data-testid="agent-status" className="max-h-96 overflow-auto whitespace-pre-wrap rounded border p-3 text-xs">{JSON.stringify(result, null, 2)}</pre>
    </> : null}
  </section>;
}
