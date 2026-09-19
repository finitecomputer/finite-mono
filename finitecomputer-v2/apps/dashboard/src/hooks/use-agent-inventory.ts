"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { HostedHermesStatusError } from "@/lib/hosted-hermes-status";

/** The server keys the component by account + organization + selected agent.
 * Each read gets fresh authorization. Nothing survives in a shared cache. */
export function useAgentInventory<T>(runtimeId: string, read: (id: string, signal: AbortSignal) => Promise<T>) {
  const [data, setData] = useState<T | null>(null);
  const [loadedAt, setLoadedAt] = useState<string | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const pending = useRef<AbortController | null>(null);
  const refresh = useCallback(async () => {
    pending.current?.abort();
    const controller = new AbortController();
    pending.current = controller;
    setBusy(true);
    setError(null);
    try {
      const result = await read(runtimeId, controller.signal);
      if (controller.signal.aborted) return;
      setData(result);
      setLoadedAt(new Date().toISOString());
      setRevision(value => value + 1);
    } catch (caught) {
      if (controller.signal.aborted) return;
      if (caught instanceof HostedHermesStatusError && caught.kind !== "request") {
        setData(null);
        setLoadedAt(null);
        setRevision(value => value + 1);
      }
      setError(caught instanceof Error ? caught.message : "The list is unavailable. Try again.");
    } finally {
      if (!controller.signal.aborted) setBusy(false);
    }
  }, [read, runtimeId]);
  useEffect(() => {
    void refresh();
    return () => pending.current?.abort();
  }, [refresh]);
  return { data, loadedAt, busy, error, revision, refresh };
}
