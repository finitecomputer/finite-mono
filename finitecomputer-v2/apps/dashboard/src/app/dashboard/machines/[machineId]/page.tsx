import Link from "next/link";
import { redirect } from "next/navigation";
import {
  HeartPulseIcon,
  MessageSquareIcon,
  RotateCcwIcon,
  StopCircleIcon,
  Trash2Icon,
} from "lucide-react";

import {
  recoverCoreRuntimeAction,
  restartCoreRuntimeAction,
  stopCoreRuntimeAction,
} from "@/app/actions";
import { FormActionButton } from "@/components/form-action-button";
import { AgentHeroCard } from "@/components/agent-hero-card";
import { ConfirmSubmitButton } from "@/components/admin-ops-forms";
import { Button } from "@/components/ui/button";
import {
  loadDashboardMachineAccess,
  type DashboardMachineAccess,
} from "@/lib/dashboard-machine-access";
import {
  coreProjectSupportsHostedRecovery,
  coreProjectSupportsHostedRestart,
  coreProjectSupportsHostedStop,
  type CoreRuntimeStatus,
} from "@/lib/core-client";

type RelayOverviewState = {
  state: "connected" | "stale" | "missing" | "unavailable";
  description: string;
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
  if (access.machineId !== machineId) {
    const destination = new URL(
      `/dashboard/machines/${encodeURIComponent(access.machineId)}`,
      "https://finite.invalid"
    );
    const removal = firstSearchParam(query.removal);
    if (removal) destination.searchParams.set("removal", removal);
    redirect(`${destination.pathname}${destination.search}`);
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
  const activeRetirement =
    access.coreProject.active_runtime_control?.kind === "destroy"
      ? access.coreProject.active_runtime_control
      : null;
  const overview = activeRetirement
    ? {
        state: "stale" as const,
        description: activeRetirement.retrying
          ? "Retirement is retrying safely. Your agent data remains retained."
          : "Your agent is being retired safely.",
      }
    : coreRuntimeOverview(access.coreProject.runtime?.runtime_status ?? "unknown");
  const prismState = prismStateForRelay(overview);
  const canRestartRuntime = coreProjectSupportsHostedRestart(access.coreProject);
  const canStopRuntime = coreProjectSupportsHostedStop(access.coreProject);
  // Chat recovery remains operator maintenance. Retirement is owner-facing
  // only when both product and persisted Runtime capability gates are open.
  const isAdminViewer = Boolean(access.viewer.isAdmin);
  const canRecoverRuntime =
    isAdminViewer && coreProjectSupportsHostedRecovery(access.coreProject);
  const canRetireRuntime = access.canRetireRuntime && !activeRetirement;

  return (
    <div className="space-y-6">
      {removalResult === "failed" ? (
        <section
          className="rounded-xl border border-destructive/30 bg-destructive/10 p-4 text-sm"
          role="alert"
        >
          We couldn&apos;t start retirement. Your agent was not offboarded; please try again.
        </section>
      ) : null}
      {removalResult === "unavailable" ? (
        <section
          className="rounded-xl border border-amber-500/40 bg-amber-500/10 p-4 text-sm"
          role="status"
        >
          This agent cannot be retired from the dashboard.
        </section>
      ) : null}
      <AgentHeroCard
        name={access.displayName}
        description={overview.description}
        state={prismState}
        actions={
          <>
              {canRestartRuntime ? (
                <form action={restartCoreRuntimeAction}>
                  <input type="hidden" name="machineId" value={access.machineId} />
                  <input type="hidden" name="redirectPath" value={`/dashboard/machines/${access.machineId}`} />
                  <FormActionButton variant="outline" pendingLabel="Restarting...">
                    <RotateCcwIcon />
                    Restart agent
                  </FormActionButton>
                </form>
              ) : null}
              {canStopRuntime ? (
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
          </>
        }
      />
      {canRecoverRuntime ? (
        <section className="rounded-xl border bg-card p-5">
          <h2 className="font-semibold">Chat recovery</h2>
          <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
            Restarts and reconciles this agent&apos;s known-good chat services. This does
            not restore a backup or delete chat data.
          </p>
          <form action={recoverCoreRuntimeAction} className="mt-4">
            <input type="hidden" name="machineId" value={access.machineId} />
            <input
              type="hidden"
              name="redirectPath"
              value={`/dashboard/machines/${access.machineId}`}
            />
            <FormActionButton variant="outline" pendingLabel="Recovering chat...">
              <HeartPulseIcon />
              Recover chat
            </FormActionButton>
          </form>
        </section>
      ) : null}
      {activeRetirement ? (
        <section className="rounded-xl border border-amber-500/40 bg-amber-500/10 p-5">
          <h2 className="font-semibold">Retiring agent</h2>
          <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
            {activeRetirement.retrying
              ? "The last attempt was interrupted and is retrying the same retirement request. Your agent data remains retained until recovery verification succeeds, and the agent stays visible until retirement commits."
              : "The agent is stopping and creating a verified support-held recovery snapshot. It stays visible until that snapshot is proven and compute removal completes."}
          </p>
        </section>
      ) : null}
      {canRetireRuntime ? (
        <section className="rounded-xl border border-destructive/30 bg-destructive/5 p-5">
          <h2 className="font-semibold">Retire this agent</h2>
          <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
            Retirement stops this agent, removes it from your dashboard, and releases its
            active slot after a verified support-held recovery snapshot is created. There is
            no self-service restore or undo. Your original agent data is retained.
          </p>
          <form
            action={`/dashboard/machines/${encodeURIComponent(access.machineId)}/remove`}
            method="post"
            className="mt-4"
          >
            <ConfirmSubmitButton
              variant="destructive"
              pendingLabel="Starting retirement..."
              confirmMessage="Retire this agent? It will stop, leave your dashboard, and release its slot after a verified recovery snapshot is created. There is no self-service undo."
            >
              <Trash2Icon />
              Retire agent
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

function coreRuntimeOverview(status: CoreRuntimeStatus): RelayOverviewState {
  if (status === "online") {
    return {
      state: "connected",
      description: "Your agent is online.",
    };
  }
  if (status === "stale") {
    return {
      state: "stale",
      description: "Your agent needs attention.",
    };
  }
  if (status === "offline") {
    return {
      state: "missing",
      description: "Your agent is stopped.",
    };
  }
  return {
    state: "unavailable",
    description: "Your agent is starting.",
  };
}

function prismStateForRelay(relay: RelayOverviewState) {
  return relay.state === "connected" ? "happy" : relay.state === "stale" ? "working" : "stuck";
}
