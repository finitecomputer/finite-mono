"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import {
  ActivityIcon,
  BanIcon,
  ExternalLinkIcon,
  RotateCcwIcon,
  SearchIcon,
  ServerIcon,
  ShieldCheckIcon,
} from "lucide-react";

import {
  adminOpsRecoverRuntimeAction,
  adminOpsRevokeFinitePrivateKeyAction,
  adminOpsResetFinitePrivateWindowAction,
  adminOpsRestartRuntimeAction,
} from "@/app/actions";
import {
  AdminFinitePrivateProfileForm,
  AdminRotateKeyForm,
  ConfirmSubmitButton,
} from "@/components/admin-ops-forms";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  adminRuntimeMatchesSearch,
  adminRuntimeSupportsRecovery,
  adminRuntimeSupportsRestart,
  adminRuntimeSupportsUpgrade,
  finitePrivateAccountForProject,
  finitePrivateAssignableProfiles,
  finitePrivateGrantSummaryForRuntime,
  finitePrivateProfileLabel,
  groupAdminRuntimesByOwner,
} from "@/lib/admin-ops";
import type { RuntimeFinitePrivateGrantSummary } from "@/lib/admin-ops";
import type {
  CoreAdminRuntimeOverview,
  CoreAdminRuntimesResult,
  CoreFinitePrivateAdminAccount,
  CoreFinitePrivateAdminStateResult,
  CoreFinitePrivateApiKey,
  CoreFinitePrivateLimitProfile,
  CoreRuntimeStatus,
} from "@/lib/core-client";

type ProvisionedRuntimeEntry = {
  runtime: CoreAdminRuntimeOverview;
  finitePrivateGrant: RuntimeFinitePrivateGrantSummary | null;
};

export function AdminUsersPanel({
  result,
  finitePrivate,
}: {
  result: CoreAdminRuntimesResult;
  finitePrivate: CoreFinitePrivateAdminStateResult;
}) {
  const [query, setQuery] = useState("");
  const profiles = finitePrivateAssignableProfiles(finitePrivate.state?.profiles);
  const userEntries = useMemo(
    () =>
      groupAdminRuntimesByOwner(result.runtimes ?? []).map((group) => {
        const account =
          group.runtimes
            .map((runtime) =>
              finitePrivateAccountForProject(
                finitePrivate.state?.accounts,
                runtime.project_id
              )
            )
            .find((candidate) => candidate !== null) ?? null;
        const runtimeEntries = group.runtimes.map((runtime) => ({
          runtime,
          finitePrivateGrant: finitePrivateGrantSummaryForRuntime(
            runtime,
            finitePrivate.state,
          ),
        }));
        return {
          group,
          account,
          runtimeEntries,
          totalRuntimeCount: group.runtimes.length,
        };
      }),
    [finitePrivate.state, result.runtimes]
  );
  const filteredUserEntries = useMemo(
    () =>
      userEntries
        .map(({ group, account, runtimeEntries, totalRuntimeCount }) => ({
          group,
          runtimeEntries: runtimeEntries.filter(
            ({ runtime, finitePrivateGrant }) =>
              adminRuntimeMatchesSearch(
                runtime,
                account,
                query,
                finitePrivateGrant,
              ),
          ),
          account,
          totalRuntimeCount,
        }))
        .filter(({ runtimeEntries }) => runtimeEntries.length > 0),
    [query, userEntries]
  );
  const totalAgentCount = userEntries.reduce(
    (total, entry) => total + entry.totalRuntimeCount,
    0
  );
  const visibleAgentCount = filteredUserEntries.reduce(
    (total, entry) => total + entry.runtimeEntries.length,
    0
  );
  const isFiltered = query.trim().length > 0;

  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          <ServerIcon className="size-5" />
        </span>
        <div>
          <h2 className="ocean-utility-card__title">Users</h2>
          <p className="text-sm text-muted-foreground">
            Each account with its agents, runtime controls, Finite Private
            usage, assigned limit, and keys in one place.
          </p>
        </div>
      </div>

      {!result.configured ? (
        <div className="ocean-empty-state">
          Finite Core is not configured: {result.missing.join(", ")}.
        </div>
      ) : result.error ? (
        <div className="ocean-empty-state">{result.error}</div>
      ) : userEntries.length === 0 ? (
        <div className="ocean-empty-state">No users with provisioned agents yet.</div>
      ) : (
        <div className="grid gap-3">
          <div className="grid gap-2 md:grid-cols-[minmax(0,1fr)_auto] md:items-end">
            <div className="grid gap-1.5">
              <label
                htmlFor="adminAgentFilter"
                className="text-xs font-semibold uppercase tracking-wide text-muted-foreground"
              >
                Filter agents
              </label>
              <div className="relative">
                <SearchIcon
                  className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
                  aria-hidden
                />
                <Input
                  id="adminAgentFilter"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  className="pl-8"
                  placeholder="Agent, user, Kata box, runtime, grant, key, or profile"
                  type="search"
                />
              </div>
            </div>
            <div className="text-xs text-muted-foreground md:pb-2">
              Showing {visibleAgentCount} of {totalAgentCount} agent
              {totalAgentCount === 1 ? "" : "s"}
            </div>
          </div>

          {filteredUserEntries.length === 0 ? (
            <div className="ocean-empty-state">No agents match that filter.</div>
          ) : (
            filteredUserEntries.map(({ group, account, runtimeEntries, totalRuntimeCount }) => (
              <ProvisionedUserCard
                key={group.key}
                email={group.email}
                runtimeEntries={runtimeEntries}
                totalRuntimeCount={totalRuntimeCount}
                isFiltered={isFiltered}
                finitePrivateAccount={account}
                finitePrivateDetailsAvailable={finitePrivate.state !== null}
                profiles={profiles}
              />
            ))
          )}
        </div>
      )}
    </section>
  );
}

function ProvisionedUserCard({
  email,
  runtimeEntries,
  totalRuntimeCount,
  isFiltered,
  finitePrivateAccount,
  finitePrivateDetailsAvailable,
  profiles,
}: {
  email: string | null;
  runtimeEntries: ProvisionedRuntimeEntry[];
  totalRuntimeCount: number;
  isFiltered: boolean;
  finitePrivateAccount: CoreFinitePrivateAdminAccount | null;
  finitePrivateDetailsAvailable: boolean;
  profiles: CoreFinitePrivateLimitProfile[];
}) {
  const runtimeFinitePrivateGrant =
    runtimeEntries
      .map((entry) => entry.finitePrivateGrant)
      .find((candidate) => candidate !== null) ?? null;
  const countText =
    isFiltered && totalRuntimeCount !== runtimeEntries.length
      ? `${runtimeEntries.length} of ${totalRuntimeCount} agent${totalRuntimeCount === 1 ? "" : "s"}`
      : `${runtimeEntries.length} agent${runtimeEntries.length === 1 ? "" : "s"}`;
  return (
    <article className="grid gap-4 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h3 className="font-semibold text-foreground">{email ?? "Unknown account"}</h3>
          <p className="text-xs text-muted-foreground">{countText}</p>
        </div>
        {finitePrivateAccount ? (
          <span className="rounded-full border border-emerald-400/40 px-2 py-0.5 text-xs text-emerald-400">
            Finite Private {finitePrivateAccount.grant.status}
          </span>
        ) : runtimeFinitePrivateGrant ? (
          <span className="rounded-full border border-emerald-400/40 px-2 py-0.5 text-xs text-emerald-400">
            Finite Private {runtimeFinitePrivateGrant.grantStatus}
          </span>
        ) : (
          <span className="rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">
            {finitePrivateDetailsAvailable
              ? "No Finite Private grant"
              : "Finite Private details unavailable"}
          </span>
        )}
      </div>
      <div className="grid gap-3">
        {runtimeEntries.map(({ runtime, finitePrivateGrant }) => (
          <ProvisionedRuntimeRow
            key={runtime.agent_runtime_id}
            runtime={runtime}
            finitePrivateAccount={finitePrivateAccount}
            finitePrivateGrant={finitePrivateGrant}
          />
        ))}
      </div>
      {finitePrivateAccount ? (
        <FinitePrivateAccountControls
          account={finitePrivateAccount}
          profiles={profiles}
        />
      ) : null}
    </article>
  );
}

function ProvisionedRuntimeRow({
  runtime,
  finitePrivateAccount,
  finitePrivateGrant,
}: {
  runtime: CoreAdminRuntimeOverview;
  finitePrivateAccount: CoreFinitePrivateAdminAccount | null;
  finitePrivateGrant: RuntimeFinitePrivateGrantSummary | null;
}) {
  const canRestart = adminRuntimeSupportsRestart(runtime);
  const canRecover = adminRuntimeSupportsRecovery(runtime);
  const canUpgrade = adminRuntimeSupportsUpgrade(runtime);
  const resetGrantId =
    finitePrivateGrant?.grantId ?? finitePrivateAccount?.grant.id ?? null;
  const resetConfirmMessage = finitePrivateGrant
    ? `Reset burst usage for ${runtime.project_display_name} (${runtime.source_machine_id}) grant ${finitePrivateGrant.grantId}? Current usage is ${formatUsageUnits(finitePrivateGrant.currentWindowUsedUnits)} units.`
    : finitePrivateAccount
      ? `Reset Finite Private usage for ${runtime.project_display_name} (${finitePrivateAccount.email})?`
      : "";

  return (
    <div className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-black/10 p-3 md:grid-cols-[minmax(0,1fr)_auto]">
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
          <span className="truncate font-mono">
            {runtime.source_host_id} / {runtime.source_machine_id}
          </span>
          <span className="truncate font-mono">
            artifact {runtime.runtime_artifact_version_label ?? runtime.runtime_artifact_id ?? "none"}
          </span>
          <span>
            health {runtimeHealthLabel(runtime)}
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
        {finitePrivateGrant ? (
          <RuntimeFinitePrivateSummary finitePrivate={finitePrivateGrant} />
        ) : null}
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
          {resetGrantId ? (
            <form action={adminOpsResetFinitePrivateWindowAction}>
              <input type="hidden" name="grantId" value={resetGrantId} />
              <ConfirmSubmitButton
                variant="outline"
                size="sm"
                pendingLabel="Resetting..."
                confirmMessage={resetConfirmMessage}
              >
                <RotateCcwIcon />
                Reset usage
              </ConfirmSubmitButton>
            </form>
          ) : null}
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

function RuntimeFinitePrivateSummary({
  finitePrivate,
}: {
  finitePrivate: RuntimeFinitePrivateGrantSummary;
}) {
  return (
    <div className="mt-3 rounded-[var(--radius-card-inner)] border border-border bg-black/10 p-3">
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
  );
}

function FinitePrivateAccountControls({
  account,
  profiles,
}: {
  account: CoreFinitePrivateAdminAccount;
  profiles: CoreFinitePrivateLimitProfile[];
}) {
  const grant = account.grant;
  return (
    <div className="grid gap-3 border-t border-border pt-4">
      <div>
        <div className="flex flex-wrap items-center gap-2 font-semibold text-foreground">
          <ShieldCheckIcon className="size-4" />
          Finite Private
        </div>
        <p className="mt-1 text-xs text-muted-foreground">
          {finitePrivateProfileLabel(grant.limit_profile_id)} · {grant.current_window_used_units.toLocaleString()} units used
        </p>
      </div>

      {profiles.length > 0 ? (
        <AdminFinitePrivateProfileForm
          grantId={grant.id}
          currentProfileId={grant.limit_profile_id}
          profiles={profiles}
        />
      ) : null}

      <div className="grid gap-2">
        <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Keys
        </div>
        {account.apiKeys.length === 0 ? (
          <div className="text-sm text-muted-foreground">No keys assigned to this account.</div>
        ) : (
          account.apiKeys.map((apiKey) => (
            <FinitePrivateAccountKey
              key={apiKey.id}
              apiKey={apiKey}
              account={account}
            />
          ))
        )}
      </div>
    </div>
  );
}

function FinitePrivateAccountKey({
  apiKey,
  account,
}: {
  apiKey: CoreFinitePrivateApiKey;
  account: CoreFinitePrivateAdminAccount;
}) {
  const project = account.projects.find((candidate) => candidate.id === apiKey.project_id);
  return (
    <div className="flex flex-wrap items-start justify-between gap-3 rounded-[var(--radius-card-inner)] border border-border bg-black/10 p-3">
      <div className="min-w-0 text-xs text-muted-foreground">
        <div className="flex flex-wrap items-center gap-2">
          <span className="truncate font-mono text-foreground">{apiKey.id}</span>
          <span className="rounded-full border border-border px-2 py-0.5">{apiKey.status}</span>
        </div>
        <div className="mt-1">
          {project ? `${project.displayName} key` : "Account key"}
        </div>
      </div>
      {apiKey.status === "active" ? (
        <div className="flex flex-wrap items-start gap-2">
          <AdminRotateKeyForm keyId={apiKey.id} />
          <form action={adminOpsRevokeFinitePrivateKeyAction}>
            <input type="hidden" name="keyId" value={apiKey.id} />
            <ConfirmSubmitButton
              variant="outline"
              size="sm"
              pendingLabel="Revoking..."
              confirmMessage="Revoke this Finite Private key? Anything using it stops working."
            >
              <BanIcon />
              Revoke
            </ConfirmSubmitButton>
          </form>
        </div>
      ) : null}
    </div>
  );
}

function runtimeStatusPillClass(status: CoreRuntimeStatus) {
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

/**
 * The evidence behind the derived status pill: the latest health report's
 * state and when the runner observed it (absolute, like every other admin
 * timestamp), plus the raw lifecycle latch whenever it differs.
 */
function runtimeHealthLabel(runtime: CoreAdminRuntimeOverview) {
  const health = runtime.runtime_health;
  const observedAt = health?.observed_at ?? health?.reported_at;
  const parts = [
    `${health?.status ?? "unknown"}${health?.reason ? ` (${health.reason})` : ""}`,
    observedAt ? `observed ${formatAdminDate(observedAt)}` : "never reported",
  ];
  if (runtime.lifecycle_status && runtime.lifecycle_status !== runtime.runtime_status) {
    parts.push(`lifecycle ${runtime.lifecycle_status}`);
  }
  return parts.join(", ");
}

function formatAdminDate(value: string) {
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? new Date(timestamp).toLocaleString() : value;
}

function formatUsageUnits(value: number) {
  return new Intl.NumberFormat("en-US").format(value);
}
