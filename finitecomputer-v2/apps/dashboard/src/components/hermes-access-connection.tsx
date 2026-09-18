"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { GlobeIcon, RefreshCwIcon } from "lucide-react";

import { ConnectionCard } from "@/components/connection-card";
import { Button } from "@/components/ui/button";
import {
  changeHostedHermesAccess, hostedHermesAccessApplied, HostedHermesStatusError,
  readHostedHermesAccess, type HostedHermesAccess,
} from "@/lib/hosted-hermes-status";

/** Keyed by account and runtime by the page. Reads never enable access. */
export function HermesAccessConnection({ runtimeId }: { runtimeId: string }) {
  const [access, setAccess] = useState<HostedHermesAccess | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef<AbortController | null>(null);

  const request = useCallback(async (change?: { access: HostedHermesAccess; enabled: boolean }) => {
    pending.current?.abort();
    const controller = new AbortController();
    pending.current = controller;
    setBusy(true);
    setError(null);
    try {
      const next = change
        ? await changeHostedHermesAccess(change.access, change.enabled, controller.signal)
        : await readHostedHermesAccess(runtimeId, controller.signal);
      if (!controller.signal.aborted) setAccess(next);
    } catch (caught) {
      if (controller.signal.aborted) return;
      // A timed-out write may have succeeded; refresh before making another
      // choice, and never retry a mutation with an outdated generation.
      setAccess(null);
      setError(caught instanceof HostedHermesStatusError ? caught.message : "Access could not be checked. Refresh status and try again.");
    } finally {
      if (!controller.signal.aborted) setBusy(false);
    }
  }, [runtimeId]);

  useEffect(() => {
    void request();
    return () => pending.current?.abort();
  }, [request]);

  const applied = access !== null && hostedHermesAccessApplied(access);
  const detail = !access ? "Check access status before making changes."
    : !access.enrolled ? "Web access is not available for this agent yet."
    : access.applyStatus === "error" ? "The agent could not apply this change. Refresh status to check again."
    : !applied ? `${access.enabled ? "Turning on" : "Turning off"} web access. Refresh status in a moment.`
    : access.enabled ? "Web access is on for your account."
    : "Web access is off. Turn it on to view this agent’s skills.";

  return (
    <div id="web-access">
      <ConnectionCard
        name="Agent web access"
        icon={<GlobeIcon className="size-5" />}
        state={busy || (access?.applyStatus === "pending") ? "loading"
          : !access || !access.enrolled || access.applyStatus === "error" ? "unavailable"
          : applied && access.enabled ? "connected" : "disconnected"}
        description="Allow your signed-in account to access this agent through the dashboard, including its Skills page."
        footer={<div className="space-y-2" aria-live="polite">
          <p className="text-sm text-muted-foreground">{detail}</p>
          {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
        </div>}
      >
        <div className="flex flex-wrap justify-end gap-2">
          <Button variant="outline" disabled={busy} onClick={() => void request()}>
            <RefreshCwIcon />Refresh status
          </Button>
          <Button
            variant={access?.enabled ? "outline" : "default"}
            disabled={busy || !access || (!access.enrolled && !access.enabled)}
            onClick={() => { if (access) void request({ access, enabled: !access.enabled }); }}
          >
            {access?.enabled ? "Turn off web access" : "Turn on web access"}
          </Button>
        </div>
      </ConnectionCard>
    </div>
  );
}
