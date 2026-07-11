import Link from "next/link";
import { redirect } from "next/navigation";
import {
  MessageSquareIcon,
  RotateCcwIcon,
  StopCircleIcon,
  Trash2Icon,
} from "lucide-react";

import {
  restartCoreRuntimeAction,
  stopCoreRuntimeAction,
} from "@/app/actions";
import { FormActionButton } from "@/components/form-action-button";
import { ConfirmSubmitButton } from "@/components/admin-ops-forms";
import { StatusPrism } from "@/components/status-prism";
import { Button } from "@/components/ui/button";
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
  searchParams,
}: {
  params: Promise<{ machineId: string }>;
  searchParams: Promise<{ removal?: string | string[] }>;
}) {
  const { machineId } = await params;
  const query = await searchParams;
  const access = await loadDashboardMachineAccess(machineId, {
    coreCacheMode: "swr",
  });

  if (!access) {
    redirect("/");
  }

  return (
    <ImportedMachineOverview
      access={access}
      removalResult={firstSearchParam(query.removal)}
    />
  );
}

async function ImportedMachineOverview({
  access,
  removalResult,
}: {
  access: DashboardMachineAccess;
  removalResult: string | null;
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
      {removalResult === "failed" ? (
        <section
          className="rounded-xl border border-destructive/30 bg-destructive/10 p-4 text-sm"
          role="alert"
        >
          We couldn&apos;t remove this agent. Please try again.
        </section>
      ) : null}
      {removalResult === "unavailable" ? (
        <section
          className="rounded-xl border border-amber-500/40 bg-amber-500/10 p-4 text-sm"
          role="status"
        >
          Only active Kata agents can be removed here.
        </section>
      ) : null}
      <section className="ocean-status-card" data-cube-state={prismState}>
        <div className="ocean-status-card__inner">
          <StatusPrism state={prismState} className="justify-self-center" />
          <div className="ocean-status-card__copy">
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
      {access.canRemoveKataRuntime ? (
        <section className="rounded-xl border border-destructive/30 bg-destructive/5 p-5">
          <h2 className="font-semibold">Remove this agent</h2>
          <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
            This removes the agent&apos;s compute so you can create a new agent. Your saved
            agent data is retained.
          </p>
          <form
            action={`/dashboard/machines/${encodeURIComponent(access.machineId)}/remove`}
            method="post"
            className="mt-4"
          >
            <ConfirmSubmitButton
              variant="destructive"
              pendingLabel="Removing..."
              confirmMessage="Remove this agent's compute? Your saved agent data will be retained."
            >
              <Trash2Icon />
              Remove agent
            </ConfirmSubmitButton>
          </form>
        </section>
      ) : null}
    </div>
  );
}

function firstSearchParam(value: string | string[] | undefined) {
  return Array.isArray(value) ? (value[0] ?? null) : (value ?? null);
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
        label: "Still starting",
        description: "Your agent is still starting.",
        lastSeenAt: null,
      };
    }

    const ageMs = Date.now() - Date.parse(heartbeat.lastSeenAt);
    if (!Number.isFinite(ageMs)) {
      return {
        state: "stale",
        label: "Needs attention",
        description: "Machine status needs attention.",
        lastSeenAt: heartbeat.lastSeenAt,
      };
    }

    const lastSeenLabel = `Your agent was last active ${formatRelativeAge(ageMs)}.`;
    return {
      state: ageMs <= RELAY_FRESH_MS ? "connected" : "stale",
      label: lastSeenLabel,
      description: ageMs <= RELAY_FRESH_MS ? "Your agent is online." : lastSeenLabel,
      lastSeenAt: heartbeat.lastSeenAt,
    };
  } catch {
    return {
      state: "unavailable",
      label: "Status unavailable",
      description: "Your agent status is unavailable right now.",
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
