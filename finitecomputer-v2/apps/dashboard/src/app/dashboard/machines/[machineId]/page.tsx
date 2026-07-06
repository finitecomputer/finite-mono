import Link from "next/link";
import { redirect } from "next/navigation";
import {
  CheckCircle2Icon,
  Loader2Icon,
  LogOutIcon,
  MessageSquareIcon,
  QrCodeIcon,
  RotateCcwIcon,
  ServerCogIcon,
  StopCircleIcon,
  Trash2Icon,
  WrenchIcon,
} from "lucide-react";

import {
  destroyCoreRuntimeAction,
  recoverCoreRuntimeAction,
  restartCoreRuntimeAction,
  stopCoreRuntimeAction,
} from "@/app/actions";
import { CopyInviteButton } from "@/components/copy-invite-button";
import { FormActionButton } from "@/components/form-action-button";
import { PendingRefresh } from "@/components/pending-refresh";
import { RefreshButton } from "@/components/refresh-button";
import { SignOutLink } from "@/components/sign-out-link";
import { StatusPrism } from "@/components/status-prism";
import { Button } from "@/components/ui/button";
import {
  AGENT_INVITE_MAX_POLL_INTERVAL_MS,
  AGENT_INVITE_POLL_INTERVAL_MS,
  agentInviteWaitStampRedirectPath,
  fetchAgentInvite,
  parseAgentInviteWaitStartedAt,
  resolveAgentInviteDisplayNow,
  truncateInviteUrl,
  truncateNpub,
  type AgentInviteDisplay,
} from "@/lib/agent-invite";
import {
  loadDashboardMachineAccess,
  type DashboardMachineAccess,
} from "@/lib/dashboard-machine-access";
import {
  fetchMachineRelayHeartbeat,
  type RelayEndpointConfig,
} from "@/lib/finite-relay-client";
import { coreProjectSupportsHostedRuntimeControl } from "@/lib/core-client";
import { qrSvgModel } from "@/lib/qr-svg";

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
  searchParams: Promise<{ inviteWaitStartedAt?: string | string[] }>;
}) {
  const { machineId } = await params;
  const query = await searchParams;
  const access = await loadDashboardMachineAccess(machineId, {
    coreCacheMode: "swr",
  });

  if (!access) {
    redirect("/");
  }

  // The invite JSON is fetched server-side only; the runtime origin is never
  // reached from the browser.
  const invite = access.coreProject?.runtime
    ? await fetchAgentInvite(access.primaryUrl)
    : null;
  const inviteDisplay = invite
    ? resolveAgentInviteDisplayNow({
        invite,
        waitStartedAtMs: parseAgentInviteWaitStartedAt(
          firstSearchParam(query.inviteWaitStartedAt)
        ),
      })
    : null;
  if (inviteDisplay?.kind === "stamp-wait-start") {
    // First render with the invite still pending: stamp the wait window start
    // so the refresh poll stays bounded (same pattern as the billing sync).
    redirect(agentInviteWaitStampRedirectPath(access.machineId));
  }

  return <ImportedMachineOverview access={access} inviteDisplay={inviteDisplay} />;
}

function firstSearchParam(value: string | string[] | undefined) {
  if (Array.isArray(value)) {
    return value[0] ?? null;
  }
  return value ?? null;
}

async function ImportedMachineOverview({
  access,
  inviteDisplay,
}: {
  access: DashboardMachineAccess;
  inviteDisplay: AgentInviteDisplay | null;
}) {
  const relay = await loadRelayOverview(access.machineId, access.relayEndpoint);
  const prismState = prismStateForRelay(relay);
  const runtime = access.coreProject?.runtime ?? null;
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
              This agent is available in Finite Chat. {relay.description}
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
                <form action={recoverCoreRuntimeAction}>
                  <input type="hidden" name="machineId" value={access.machineId} />
                  <input type="hidden" name="redirectPath" value={`/dashboard/machines/${access.machineId}`} />
                  <FormActionButton variant="outline" pendingLabel="Recovering...">
                    <WrenchIcon />
                    Recover chat
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
              {canControlRuntime ? (
                <form action={destroyCoreRuntimeAction}>
                  <input type="hidden" name="machineId" value={access.machineId} />
                  <input type="hidden" name="redirectPath" value={`/dashboard/machines/${access.machineId}`} />
                  <FormActionButton variant="destructive" pendingLabel="Destroying...">
                    <Trash2Icon />
                    Destroy
                  </FormActionButton>
                </form>
              ) : null}
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

      {inviteDisplay ? (
        <AgentInviteCard display={inviteDisplay} machineId={access.machineId} />
      ) : null}

      <div className="ocean-action-grid">
        <div className="ocean-action-card">
          <span className="ocean-action-card__icon">
            <MessageSquareIcon className="size-5" />
          </span>
          <span className="ocean-action-card__body">
            <span className="ocean-action-card__title">Finite Chat</span>
            <span className="ocean-action-card__description">
              Use the native iOS app for encrypted chat and agent-managed configuration.
            </span>
          </span>
        </div>
        <div className="ocean-action-card">
          <span className="ocean-action-card__icon">
            <WrenchIcon className="size-5" />
          </span>
          <span className="ocean-action-card__body">
            <span className="ocean-action-card__title">Runtime recovery</span>
            <span className="ocean-action-card__description">
              Restart normally first; recover chat only when Finite Chat or generated Hermes config is broken.
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

function AgentInviteCard({
  display,
  machineId,
}: {
  display: AgentInviteDisplay;
  machineId: string;
}) {
  // Dropping the query restamps a fresh bounded wait window.
  const recheckHref = `/dashboard/machines/${encodeURIComponent(machineId)}`;
  const paired = display.kind === "paired";

  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          {paired ? (
            <CheckCircle2Icon className="size-5" />
          ) : (
            <QrCodeIcon className="size-5" />
          )}
        </span>
        <div>
          <h2 className="ocean-utility-card__title">
            {paired ? "Paired with Finite Chat" : "Pair with Finite Chat"}
          </h2>
          <p className="text-sm text-muted-foreground">
            {paired
              ? "This agent is connected to your Finite Chat account."
              : "Scan the invite with the Finite Chat iOS app to pair this agent with your phone."}
          </p>
        </div>
      </div>

      {display.kind === "paired" ? (
        <div className="grid gap-3">
          {display.agentNpub ? (
            <>
              <div className="break-all rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-3 font-mono text-sm text-foreground">
                {truncateNpub(display.agentNpub)}
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <CopyInviteButton value={display.agentNpub} label="Copy agent npub" />
              </div>
            </>
          ) : null}
          <p className="text-sm text-muted-foreground">
            Open Finite Chat on your phone to talk to your agent.
          </p>
        </div>
      ) : null}

      {display.kind === "ready" ? (
        <div className="grid gap-4 md:grid-cols-[auto_minmax(0,1fr)] md:items-center">
          <InviteQr inviteUrl={display.inviteUrl} />
          <div className="grid gap-3">
            <div className="break-all rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-3 font-mono text-sm text-foreground">
              {truncateInviteUrl(display.inviteUrl)}
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <CopyInviteButton value={display.inviteUrl} />
              <Button asChild variant="outline" size="sm">
                <a href={display.inviteUrl}>
                  <MessageSquareIcon />
                  Open in Finite Chat
                </a>
              </Button>
            </div>
            <p className="text-sm text-muted-foreground">
              On this device? Open in Finite Chat directly. Otherwise scan the QR
              code with the app&apos;s camera.
            </p>
          </div>
        </div>
      ) : null}

      {display.kind === "waiting" ? (
        <>
          <PendingRefresh
            enabled
            intervalMs={AGENT_INVITE_POLL_INTERVAL_MS}
            maxIntervalMs={AGENT_INVITE_MAX_POLL_INTERVAL_MS}
            deadlineAtMs={display.deadlineAtMs}
          />
          <div className="ocean-agent-spinup" role="status" aria-live="polite">
            <Loader2Icon className="size-5 animate-spin" aria-hidden />
            <div>
              <strong>Preparing your invite</strong>
              <span>
                The runtime is up and its Finite Chat invite is being created —
                this usually takes a few seconds.
              </span>
            </div>
          </div>
        </>
      ) : null}

      {display.kind === "wait-timeout" ? (
        <div className="grid gap-3">
          <div className="ocean-empty-state">
            The invite is taking longer than expected. Check again in a moment,
            or restart the agent if this keeps happening.
          </div>
          <Button asChild variant="outline" className="w-fit">
            <Link href={recheckHref}>
              <RotateCcwIcon />
              Check again
            </Link>
          </Button>
        </div>
      ) : null}

      {display.kind === "error" ? (
        <div className="grid gap-3">
          <div className="ocean-empty-state">
            The runtime reported an invite problem: {display.message}
          </div>
          <RefreshButton>
            <RotateCcwIcon />
            Retry
          </RefreshButton>
        </div>
      ) : null}
    </section>
  );
}

function InviteQr({ inviteUrl }: { inviteUrl: string }) {
  const { moduleCount, path } = qrSvgModel(inviteUrl);
  const margin = 2;
  const span = moduleCount + margin * 2;

  return (
    <div className="w-fit rounded-[var(--radius-card-inner)] border border-border bg-white p-3">
      <svg
        viewBox={`${-margin} ${-margin} ${span} ${span}`}
        role="img"
        aria-label="Finite Chat invite QR code"
        className="size-44"
        shapeRendering="crispEdges"
      >
        <path d={path} fill="#000" />
      </svg>
    </div>
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
