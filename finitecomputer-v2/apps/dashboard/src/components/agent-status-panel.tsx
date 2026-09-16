"use client";

import { useEffect, useRef, useState } from "react";
import type { AgentEndpoint } from "@/lib/core-client";
import { readAgentSkills, readAgentStatus, transportControl, type AgentSkill } from "@/lib/agent-api-client";
import { Button } from "@/components/ui/button";

export function AgentStatusPanel({ projectId }: { projectId: string }) {
  const [binding, setBinding] = useState<AgentEndpoint | null>(null);
  const [result, setResult] = useState<unknown>(null);
  const [skills, setSkills] = useState<AgentSkill[] | null>(null);
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
    setBusy(true); setError(""); setResult(null); setSkills(null);
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
  async function check(kind: "status" | "skills") {
    setBusy(true); setError(""); setResult(null); setSkills(null);
    const controller = new AbortController(); active.current = controller;
    try {
      if (kind === "skills") {
        const result = await readAgentSkills(projectId, controller.signal);
        if (!controller.signal.aborted) setSkills(result);
      } else {
        const result = await readAgentStatus(projectId, controller.signal);
        if (!controller.signal.aborted) setResult(result);
      }
    }
    catch (e) { if (!controller.signal.aborted) setError(e instanceof Error ? e.message : "Status unavailable"); }
    finally { if (!controller.signal.aborted) setBusy(false); active.current = null; }
  }
  return <section className="space-y-4 rounded-xl border bg-card p-5">
    <h1 className="text-xl font-semibold">Agent status</h1>
    <p className="text-sm text-muted-foreground">Internal preview. Read live status and installed skills directly from this agent’s Hermes server.</p>
    <p>Hosted access: {binding ? (binding.enabled ? "Enabled" : "Disabled") : "Unavailable"}</p>
    <div className="flex gap-3">
      <Button variant="outline" disabled={!binding || busy} onClick={toggle}>
        {binding?.enabled ? "Disable hosted access" : "Enable hosted access"}
      </Button>
      <Button disabled={!binding?.enabled || busy} onClick={() => check("status")}>
        {busy ? "Working…" : "Check agent status"}
      </Button>
      <Button disabled={!binding?.enabled || busy} onClick={() => check("skills")}>
        Read agent skills
      </Button>
    </div>
    {skills !== null ? <div data-testid="agent-skills">
      <p role="status">{skills.length} skills received from this agent</p>
      {skills.length === 0 ? <p>No skills found in this Hermes profile.</p> : (
        <ul className="mt-3 max-h-96 space-y-3 overflow-auto">
          {skills.map((skill, index) => <li key={`${skill.name}:${index}`} className="rounded border p-3">
            <p className="font-medium">{skill.name}</p>
            <p className="text-sm text-muted-foreground">{skill.description}</p>
            <p className="text-xs text-muted-foreground">{skill.provenance} · {skill.enabled ? "Enabled" : "Disabled"}</p>
          </li>)}
        </ul>
      )}
    </div> : null}
    {error ? <p role="alert">{error}</p> : null}
    {result !== null ? <>
      <p role="status">Live Hermes status received</p>
      <pre data-testid="agent-status" className="max-h-96 overflow-auto whitespace-pre-wrap rounded border p-3 text-xs">{JSON.stringify(result, null, 2)}</pre>
    </> : null}
  </section>;
}
