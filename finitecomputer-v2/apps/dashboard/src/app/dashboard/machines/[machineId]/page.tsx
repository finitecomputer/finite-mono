import Link from "next/link";
import { redirect } from "next/navigation";
import {
  LogOutIcon,
  MessageSquareIcon,
  RotateCcwIcon,
  ServerCogIcon,
  StopCircleIcon,
} from "lucide-react";

import {
  restartCoreRuntimeAction,
  stopCoreRuntimeAction,
} from "@/app/actions";
import { CopyButton } from "@/components/copy-button";
import { FormActionButton } from "@/components/form-action-button";
import { SignOutLink } from "@/components/sign-out-link";
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
  const runtime = access.coreProject?.runtime ?? null;
  const overview = runtime
    ? coreRuntimeOverview(runtime.host_facts.runtime_status, runtime.updated_at)
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
              This agent is available in Finite Chat. {overview.description}
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
                  Open web chat
                </Link>
              </Button>
              <Button asChild variant="secondary">
                <SignOutLink>
                  Sign out
                  <LogOutIcon />
                </SignOutLink>
              </Button>
            </div>
          </div>
        </div>
      </section>

      {agentNpub ? <AgentContactCard agentNpub={agentNpub} /> : null}

      <div className="ocean-action-grid">
        <div className="ocean-action-card">
          <span className="ocean-action-card__icon">
            <MessageSquareIcon className="size-5" />
          </span>
          <span className="ocean-action-card__body">
            <span className="ocean-action-card__title">Finite Chat</span>
            <span className="ocean-action-card__description">
              Open the dashboard Hosted Web Device now; Electron and native clients can join later.
            </span>
          </span>
        </div>
        <div className="ocean-action-card">
          <span className="ocean-action-card__icon">
            <RotateCcwIcon className="size-5" />
          </span>
          <span className="ocean-action-card__body">
            <span className="ocean-action-card__title">Runtime restart</span>
            <span className="ocean-action-card__description">
              Generic restart preserves the agent&apos;s durable state without adding a chat-specific control path.
            </span>
          </span>
        </div>
      </div>
      {runtime ? (
        <RuntimeFactsCard access={access} />
      ) : null}
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
          <h2 className="ocean-utility-card__title">Agent identity</h2>
          <p className="text-sm text-muted-foreground">
            Hosted Web, Electron, and native clients are independent Devices
            that start a chat with this Agent Principal.
          </p>
        </div>
      </div>
      <div className="grid gap-3">
        <div className="break-all rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-3 font-mono text-sm text-foreground">
          {truncateNpub(agentNpub)}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <CopyButton value={agentNpub} label="Copy agent npub" />
        </div>
      </div>
    </section>
  );
}

function RuntimeFactsCard({ access }: { access: DashboardMachineAccess }) {
  const runtime = access.coreProject?.runtime;
  if (!runtime) {
    return null;
  }
  const facts = runtime.host_facts;
  const safeUrls = facts.published_app_urls.filter(safeHttpUrl);
  const rows = [
    ["Project", access.coreProject?.project.id],
    ["Runtime", runtime.id],
    ["Source host", runtime.source_host_id],
    ["Source machine", runtime.source_machine_id],
    ["Runtime artifact", runtime.runtime_artifact_id],
    ["State schema", runtime.state_schema_version],
    ["Runtime host", facts.runtime_host],
    ["Status", facts.runtime_status],
    ["Hermes", facts.hermes_available === false ? "unavailable" : "available"],
    ["Inference", facts.active_inference_profile],
    ["Updated", runtime.updated_at],
  ];

  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          <ServerCogIcon className="size-5" />
        </span>
        <div>
          <h2 className="ocean-utility-card__title">Runtime facts</h2>
          <p className="text-sm text-muted-foreground">
            Plaintext operational state for support and recovery.
          </p>
        </div>
      </div>

      <dl className="grid gap-2 text-sm md:grid-cols-2">
        {rows.map(([label, value]) => (
          <div
            key={label}
            className="grid gap-1 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-3"
          >
            <dt className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">
              {label}
            </dt>
            <dd className="break-all font-mono text-foreground">{value || "unknown"}</dd>
          </div>
        ))}
      </dl>

      {safeUrls.length ? (
        <div className="grid gap-2">
          <div className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">
            Public endpoints
          </div>
          <div className="grid gap-2">
            {safeUrls.map((url) => (
              <a
                key={url}
                href={url}
                target="_blank"
                rel="noreferrer"
                className="break-all rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-3 font-mono text-sm text-foreground hover:underline"
              >
                {url}
              </a>
            ))}
          </div>
        </div>
      ) : null}
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
      label: "Runtime online.",
      description: "Runtime is online.",
      lastSeenAt: updatedAt,
    };
  }
  if (status === "stale") {
    return {
      state: "stale",
      label: "Runtime needs attention.",
      description: "Runtime needs attention.",
      lastSeenAt: updatedAt,
    };
  }
  if (status === "offline") {
    return {
      state: "missing",
      label: "Runtime stopped.",
      description: "Runtime is stopped.",
      lastSeenAt: updatedAt,
    };
  }
  return {
    state: "unavailable",
    label: "Runtime status pending.",
    description: "Runtime status is not known yet.",
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

function safeHttpUrl(value: string) {
  try {
    const parsed = new URL(value);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}
