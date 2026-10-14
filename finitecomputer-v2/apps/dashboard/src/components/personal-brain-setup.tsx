"use client";

import { useEffect, useRef, useState } from "react";
import { PersonalBrainSetupError, setUpPersonalBrain } from "@/lib/personal-brain-setup-flow";

/** One setup at a time for the mounted agent scope. Unmounting (agent or
 * account switch) aborts the browser side and ignores late results. */
export function usePersonalBrainSetup(runtimeId: string, onChanged: () => void) {
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef<AbortController | null>(null);
  useEffect(() => () => pending.current?.abort(), []);

  async function start() {
    if (pending.current) return;
    const controller = new AbortController();
    pending.current = controller;
    setRunning(true);
    setError(null);
    try {
      await setUpPersonalBrain(runtimeId, controller.signal);
      if (!controller.signal.aborted) onChanged();
    } catch (caught) {
      if (controller.signal.aborted) return;
      setError(caught instanceof Error ? caught.message : "Your Personal Brain could not be set up right now.");
      if (caught instanceof PersonalBrainSetupError && caught.mayExist) onChanged();
    } finally {
      pending.current = null;
      if (!controller.signal.aborted) setRunning(false);
    }
  }

  return { running, error, start: () => void start() };
}

/** The quiet Brain page row shown when this agent has other brains but no
 * Personal Brain of its own. */
export function PersonalBrainSetupRow({ runtimeId, agentName, onChanged }: {
  runtimeId: string; agentName: string; onChanged: () => void;
}) {
  const setup = usePersonalBrainSetup(runtimeId, onChanged);
  return <div className="flex flex-wrap items-center gap-2 py-3 text-sm text-muted-foreground">
    <span>{agentName} doesn’t have a Personal Brain yet.</span>
    <button className="inline-flex items-center gap-2 rounded-md border px-3 py-2 disabled:opacity-50" type="button" disabled={setup.running} onClick={setup.start}>
      {setup.running ? "Setting up…" : "Set up Personal Brain"}
    </button>
    {setup.error && <p role="alert" className="w-full text-amber-700 dark:text-amber-400">{setup.error}</p>}
  </div>;
}
