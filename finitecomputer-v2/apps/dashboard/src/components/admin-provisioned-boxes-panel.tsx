"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import {
  ActivityIcon,
  ExternalLinkIcon,
  RotateCcwIcon,
  SearchIcon,
  ServerIcon,
  ShieldCheckIcon,
} from "lucide-react";

import {
  adminOpsRecoverRuntimeAction,
  adminOpsResetFinitePrivateWindowAction,
  adminOpsRestartRuntimeAction,
} from "@/app/actions";
import { ConfirmSubmitButton } from "@/components/admin-ops-forms";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  adminRuntimeMatchesSearch,
  finitePrivateGrantSummaryForRuntime,
  type AdminOpsFinitePrivateState,
  type AdminOpsRuntime,
  type RuntimeFinitePrivateGrantSummary,
} from "@/lib/admin-ops";

export function AdminProvisionedBoxesPanel({
  result,
  finitePrivateState,
}: {
  result: {
    configured: boolean;
    missing: string[];
    runtimes: AdminOpsRuntime[] | null;
    error: string | null;
  };
  finitePrivateState: AdminOpsFinitePrivateState | null;
}) {
  const [query, setQuery] = useState("");
  const runtimeEntries = useMemo(
    () =>
      (result.runtimes ?? []).map((runtime) => ({
        runtime,
        finitePrivate: finitePrivateGrantSummaryForRuntime(runtime, finitePrivateState),
      })),
    [finitePrivateState, result.runtimes],
  );
  const filteredEntries = runtimeEntries.filter((entry) =>
    adminRuntimeMatchesSearch(entry.runtime, entry.finitePrivate, query),
  );

  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          <ServerIcon className="size-5" />
        </span>
        <div>
          <h2 className="ocean-utility-card__title">Provisioned boxes</h2>
          <p className="text-sm text-muted-foreground">
            Every agent runtime Core knows about, with restart, recovery, and
            usage reset controls.
          </p>
        </div>
      </div>

      {!result.configured ? (
        <div className="ocean-empty-state">
          Finite Core is not configured: {result.missing.join(", ")}.
        </div>
      ) : result.error ? (
        <div className="ocean-empty-state">{result.error}</div>
      ) : !result.runtimes || result.runtimes.length === 0 ? (
        <div className="ocean-empty-state">No provisioned boxes yet.</div>
      ) : (
        <div className="grid gap-3">
          <label className="grid gap-2 text-sm text-muted-foreground">
            <span className="font-medium text-foreground">Filter agents</span>
            <span className="relative">
              <SearchIcon className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                type="search"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Agent name, owner, Kata box, runtime, grant, or key"
                className="pl-8"
              />
            </span>
          </label>
          {filteredEntries.length === 0 ? (
            <div className="ocean-empty-state">No provisioned boxes match that filter.</div>
          ) : (
            filteredEntries.map((entry) => (
              <ProvisionedBoxRow
                key={entry.runtime.agent_runtime_id}
                runtime={entry.runtime}
                finitePrivate={entry.finitePrivate}
              />
            ))
          )}
        </div>
      )}
    </section>
  );
}

function runtimeStatusPillClass(status: string) {
  if (status === "online") {
    return "border-emerald-400/40 text-emerald-400";
  }
  if (status === "offline") {
    return "border-rose-400/40 text-rose-400";
  }
  if (status === "stale") {
    return "border-amber-400/40 text-amber-400";
  }
  return "border-border text-muted-foreground";
}

function heartbeatLabel(lastHeartbeatAt: string | null | undefined) {
  if (!lastHeartbeatAt) {
    return "never";
  }
  const timestamp = Date.parse(lastHeartbeatAt);
  return Number.isFinite(timestamp) ? formatAdminDate(lastHeartbeatAt) : "unknown";
}

function ProvisionedBoxRow({
  runtime,
  finitePrivate,
}: {
  runtime: AdminOpsRuntime;
  finitePrivate: RuntimeFinitePrivateGrantSummary | null;
}) {
  const canRestart = runtimeSupports(runtime, "restart");
  const canRecover = runtimeSupports(runtime, "recover_known_good_chat");
  const canUpgrade = runtimeSupports(runtime, "runtime_upgrade");

  return (
    <div className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4 md:grid-cols-[minmax(0,1fr)_auto]">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <span className="truncate font-semibold text-foreground">
            {runtime.project_display_name}
          </span>
          <span
            className={`rounded-full border px-2 py-0.5 text-xs ${runtimeStatusPillClass(runtime.runtime_status)}`}
          >
            {runtime.runtime_status}
          </span>
          {!runtime.runtime_link_active ? (
            <span className="rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">
              unlinked
            </span>
          ) : null}
        </div>
        <div className="mt-1 grid gap-1 text-xs text-muted-foreground">
          <span className="truncate">owner {runtime.owner_email ?? "unknown"}</span>
          <span className="truncate font-mono">
            {runtime.source_host_id} / {runtime.source_machine_id}
          </span>
          <span className="truncate font-mono">
            artifact {runtime.runtime_artifact_version_label ?? runtime.runtime_artifact_id ?? "none"}
          </span>
          <span>
            heartbeat {heartbeatLabel(runtime.last_heartbeat_at)}
            {" · "}
            hermes {runtime.hermes_available == null ? "unknown" : runtime.hermes_available ? "yes" : "no"}
            {" · "}
            {runtime.active_finite_private_key_count} active FP key
            {runtime.active_finite_private_key_count === 1 ? "" : "s"}
          </span>
          {runtime.published_app_urls.length > 0 ? (
            <span className="flex flex-wrap items-center gap-2">
              {runtime.published_app_urls.map((url) => (
                <a
                  key={url}
                  className="inline-flex items-center gap-1 truncate underline"
                  href={url}
                  target="_blank"
                  rel="noreferrer"
                >
                  <ExternalLinkIcon className="size-3" aria-hidden />
                  {url}
                </a>
              ))}
            </span>
          ) : null}
        </div>
        <RuntimeFinitePrivatePanel runtime={runtime} finitePrivate={finitePrivate} />
      </div>
      <div className="grid items-start gap-2">
        <div className="flex flex-wrap items-start gap-2">
          <form action={adminOpsRestartRuntimeAction}>
            <input type="hidden" name="projectId" value={runtime.project_id} />
            <ConfirmSubmitButton
              variant="outline"
              size="sm"
              pendingLabel="Restarting..."
              disabled={!canRestart}
              confirmMessage={`Restart ${runtime.project_display_name} (${runtime.source_machine_id})?`}
            >
              <RotateCcwIcon />
              Restart
            </ConfirmSubmitButton>
          </form>
          <form action={adminOpsRecoverRuntimeAction}>
            <input type="hidden" name="projectId" value={runtime.project_id} />
            <ConfirmSubmitButton
              variant="outline"
              size="sm"
              pendingLabel="Recovering..."
              disabled={!canRecover}
              confirmMessage={`Recover known-good chat runtime for ${runtime.project_display_name}?`}
            >
              <ActivityIcon />
              Recover
            </ConfirmSubmitButton>
          </form>
        </div>
        {canUpgrade ? (
          <Button asChild variant="outline" size="sm" className="w-fit">
            <Link
              href={{
                pathname: "/dashboard/admin/runtime-upgrade",
                query: { projectId: runtime.project_id },
              }}
            >
              <ActivityIcon />
              Upgrade
            </Link>
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function RuntimeFinitePrivatePanel({
  runtime,
  finitePrivate,
}: {
  runtime: AdminOpsRuntime;
  finitePrivate: RuntimeFinitePrivateGrantSummary | null;
}) {
  if (!finitePrivate) {
    return (
      <div className="mt-3 rounded-[var(--radius-card-inner)] border border-border bg-black/10 p-3 text-xs text-muted-foreground">
        No matching Finite Private grant was resolved for this agent.
      </div>
    );
  }

  return (
    <div className="mt-3 grid gap-2 rounded-[var(--radius-card-inner)] border border-border bg-black/10 p-3 md:grid-cols-[minmax(0,1fr)_auto]">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <span className="inline-flex items-center gap-1 font-semibold text-foreground">
            <ShieldCheckIcon className="size-3.5" />
            Finite Private
          </span>
          <span className="rounded-full border border-border px-2 py-0.5 text-muted-foreground">
            {finitePrivate.matchScope}-matched
          </span>
          <span className="rounded-full border border-border px-2 py-0.5 text-muted-foreground">
            {finitePrivate.grantStatus}
          </span>
        </div>
        <div className="mt-1 grid gap-1 text-xs text-muted-foreground">
          <span className="truncate font-mono">grant {finitePrivate.grantId}</span>
          <span className="truncate font-mono">key {finitePrivate.keyId}</span>
          <span>
            burst usage {formatUsageUnits(finitePrivate.currentWindowUsedUnits)} units
            {finitePrivate.currentWindowStartedAt
              ? ` · window ${formatAdminDate(finitePrivate.currentWindowStartedAt)}`
              : ""}
          </span>
        </div>
      </div>
      <form action={adminOpsResetFinitePrivateWindowAction}>
        <input type="hidden" name="grantId" value={finitePrivate.grantId} />
        <ConfirmSubmitButton
          variant="outline"
          size="sm"
          className="w-fit"
          pendingLabel="Resetting..."
          confirmMessage={`Reset burst usage for ${runtime.project_display_name} (${runtime.source_machine_id}) grant ${finitePrivate.grantId}? Current usage is ${formatUsageUnits(finitePrivate.currentWindowUsedUnits)} units.`}
        >
          <RotateCcwIcon />
          Reset usage
        </ConfirmSubmitButton>
      </form>
    </div>
  );
}

function formatUsageUnits(value: number) {
  return new Intl.NumberFormat("en-US").format(value);
}

function formatAdminDate(value: string) {
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? new Date(timestamp).toLocaleString() : value;
}

function runtimeSupports(
  runtime: { runtime_capabilities?: RuntimeCapabilities | null } | null | undefined,
  operation: keyof RuntimeCapabilities,
) {
  return runtime?.runtime_capabilities?.[operation] === true;
}

type RuntimeCapabilities = {
  restart?: boolean;
  recover_known_good_chat?: boolean;
  runtime_upgrade?: boolean;
};
