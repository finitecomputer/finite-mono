import Link from "next/link";
import { redirect } from "next/navigation";
import {
  MessageSquareIcon,
  RotateCcwIcon,
  StopCircleIcon,
} from "lucide-react";

import {
  restartCoreRuntimeAction,
  stopCoreRuntimeAction,
} from "@/app/actions";
import { CopyButton } from "@/components/copy-button";
import { FormActionButton } from "@/components/form-action-button";
import { StatusPrism } from "@/components/status-prism";
import { Button } from "@/components/ui/button";
import { fetchRuntimeAgentNpub, truncateNpub } from "@/lib/agent-contact";
import {
  loadDashboardMachineAccess,
  type DashboardMachineAccess,
} from "@/lib/dashboard-machine-access";
import {
  fetchMachineRelayHeartbeat,
  type RelayEndpointConfig,
} from "@/lib/finite-relay-client";
import {
  coreProjectSupportsHostedRuntimeControl,
  type CoreRuntimeStatus,
} from "@/lib/core-client";

const RELAY_FRESH_MS = 60_000;

type RelayOverviewState = {
  state: "connected" | "stale" | "missing" | "unavailable";
  label: string;
  description: string;
  lastSeenAt: string | null;
};

export default async function MachineDetailPage({
  params,
}: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  const access = await loadDashboardMachineAccess(machineId, {
    coreCacheMode: "swr",
  });

  if (!access) {
    redirect("/");
  }

  // The runtime origin is reached server-side only. Devices use the Agent
  // Principal's npub for the one canonical MLS Add + Welcome flow.
  const agentNpub = access.coreProject?.runtime
    ? await fetchRuntimeAgentNpub(access.primaryUrl)
    : null;
  return <ImportedMachineOverview access={access} agentNpub={agentNpub} />;
}

async function ImportedMachineOverview({
  access,
  agentNpub,
}: {
  access: DashboardMachineAccess;
  agentNpub: string | null;
}) {
  const overview = access.coreProject?.runtime
    ? coreRuntimeOverview(
        access.coreProject.runtime.host_facts.runtime_status,
        access.coreProject.runtime.updated_at
      )
    : await loadRelayOverview(access.machineId, access.relayEndpoint);
  const prismState = prismStateForRelay(overview);
  const canControlRuntime = access.coreProject
    ? coreProjectSupportsHostedRuntimeControl(access.coreProject)
    : false;

  return (
    <div className="space-y-6">
      <section className="ocean-status-card" data-cube-state={prismState}>
        <div className="ocean-status-card__inner">
          <StatusPrism state={prismState} className="justify-self-center" />
          <div className="ocean-status-card__copy">
            <div className="text-xs font-semibold uppercase tracking-[0.22em] text-muted-foreground">
              {access.machineId}
            </div>
            <h1 className="ocean-status-card__title">{access.displayName}</h1>
            <p className="ocean-status-card__description">
              {overview.description}
            </p>
            <div className="ocean-status-card__actions">
              {canControlRuntime ? (
                <form action={restartCoreRuntimeAction}>
                  <input type="hidden" name="machineId" value={access.machineId} />
                  <input type="hidden" name="redirectPath" value={`/dashboard/machines/${access.machineId}`} />
                  <FormActionButton variant="outline" pendingLabel="Restarting...">
                    <RotateCcwIcon />
                    Restart agent
                  </FormActionButton>
                </form>
              ) : null}
              {canControlRuntime ? (
                <form action={stopCoreRuntimeAction}>
                  <input type="hidden" name="machineId" value={access.machineId} />
                  <input type="hidden" name="redirectPath" value={`/dashboard/machines/${access.machineId}`} />
                  <FormActionButton variant="outline" pendingLabel="Stopping...">
                    <StopCircleIcon />
                    Stop
                  </FormActionButton>
                </form>
              ) : null}
              <Button asChild variant="secondary">
                <Link href={`/dashboard/machines/${encodeURIComponent(access.machineId)}/chat`}>
                  <MessageSquareIcon />
                  Open chat
                </Link>
              </Button>
            </div>
          </div>
        </div>
      </section>

      {agentNpub ? <AgentContactCard agentNpub={agentNpub} /> : null}

    </div>
  );
}

function AgentContactCard({ agentNpub }: { agentNpub: string }) {
  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          <MessageSquareIcon className="size-5" />
        </span>
        <div>
          <h2 className="ocean-utility-card__title">Agent address</h2>
          <p className="text-sm text-muted-foreground">
            Use this address to connect another Finite Chat app.
          </p>
        </div>
      </div>
      <div className="grid gap-3">
        <div className="break-all rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-3 font-mono text-sm text-foreground">
          {truncateNpub(agentNpub)}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <CopyButton value={agentNpub} label="Copy address" />
        </div>
      </div>
    </section>
  );
}

function coreRuntimeOverview(
  status: CoreRuntimeStatus,
  updatedAt: string
): RelayOverviewState {
  if (status === "online") {
    return {
      state: "connected",
      label: "Online",
      description: "Your agent is online.",
      lastSeenAt: updatedAt,
    };
  }
  if (status === "stale") {
    return {
      state: "stale",
      label: "Needs attention",
      description: "Your agent needs attention.",
      lastSeenAt: updatedAt,
    };
  }
  if (status === "offline") {
    return {
      state: "missing",
      label: "Stopped",
      description: "Your agent is stopped.",
      lastSeenAt: updatedAt,
    };
  }
  return {
    state: "unavailable",
    label: "Starting",
    description: "Your agent is starting.",
    lastSeenAt: updatedAt,
  };
}

async function loadRelayOverview(
  machineId: string,
  endpoint?: RelayEndpointConfig | null
): Promise<RelayOverviewState> {
  try {
    const heartbeat = await fetchMachineRelayHeartbeat(machineId, endpoint);
    if (!heartbeat?.lastSeenAt) {
      return {
        state: "missing",
        label: "No relay heartbeat yet.",
        description: "Chat relay has not checked in yet.",
        lastSeenAt: null,
      };
    }

    const ageMs = Date.now() - Date.parse(heartbeat.lastSeenAt);
    if (!Number.isFinite(ageMs)) {
      return {
        state: "stale",
        label: "Relay heartbeat timestamp is invalid.",
        description: "Machine status needs attention.",
        lastSeenAt: heartbeat.lastSeenAt,
      };
    }

    const lastSeenLabel = `Relay last seen ${formatRelativeAge(ageMs)}.`;
    return {
      state: ageMs <= RELAY_FRESH_MS ? "connected" : "stale",
      label: lastSeenLabel,
      description: ageMs <= RELAY_FRESH_MS ? "Chat relay is connected." : lastSeenLabel,
      lastSeenAt: heartbeat.lastSeenAt,
    };
  } catch (error) {
    return {
      state: "unavailable",
      label: error instanceof Error ? error.message : "Relay status unavailable.",
      description: "Machine status is unavailable.",
      lastSeenAt: null,
    };
  }
}

function prismStateForRelay(relay: RelayOverviewState) {
  return relay.state === "connected" ? "happy" : relay.state === "stale" ? "working" : "stuck";
}

function formatRelativeAge(ageMs: number) {
  const seconds = Math.max(0, Math.round(ageMs / 1000));
  if (seconds < 5) {
    return "just now";
  }
  if (seconds < 60) {
    return `${seconds}s ago`;
  }
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) {
    return `${minutes}m ago`;
  }
  const hours = Math.round(minutes / 60);
  return `${hours}h ago`;
}
