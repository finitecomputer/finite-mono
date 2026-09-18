"use client";

import formStyles from "@/styles/admin-forms.module.css";
import Link from "next/link";
import { useMemo, useRef, useState } from "react";
import {
  ActivityIcon,
  BanIcon,
  RotateCcwIcon,
  SearchIcon,
  ChevronRightIcon,
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
import {
  FinitePrivateUsageProgress,
  formatWeightedTokens,
} from "@/components/finite-private-usage-progress";
import { adminUserAuditEvents, adminUserTableCells, compareAdminTableCells, type AdminTableSort } from "@/lib/admin-users-table";
import { AdminUsersToolbar } from "@/components/admin-ops-tabs";
import { AdminTableScroll } from "@/components/admin-table-scroll";
import styles from "@/styles/admin-users-table.module.css";
import { Dialog, DialogContent, DialogTitle, DialogDescription } from "@/components/ui/dialog";
import dialogStyles from "@/styles/admin-agent-dialog.module.css";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  adminRuntimeMatchesSearch,
  adminRuntimeSupportsRecovery,
  adminRuntimeSupportsRestart,
  adminRuntimeSupportsUpgrade,
  finitePrivateAccountForProject,
  finitePrivateAssignableProfiles,
  finitePrivateGrantSummaryForRuntime,
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
} from "@/lib/core-client";

type ProvisionedRuntimeEntry = {
  runtime: CoreAdminRuntimeOverview;
  finitePrivateGrant: RuntimeFinitePrivateGrantSummary | null;
  cells: ReturnType<typeof adminUserTableCells>;
  auditEvents: ReturnType<typeof adminUserAuditEvents>;
};

export function AdminUsersPanel({
  result,
  finitePrivate,
}: {
  result: CoreAdminRuntimesResult;
  finitePrivate: CoreFinitePrivateAdminStateResult;
}) {
  const [sort, setSort] = useState<AdminTableSort>({ column: "Email", direction: "ascending" });
  const [selectedAgent, setSelectedAgent] = useState<{ entry: ProvisionedRuntimeEntry; account: CoreFinitePrivateAdminAccount | null } | null>(null);
  const selectedRowRef = useRef<HTMLTableRowElement | null>(null);
  const [query, setQuery] = useState("");
  const [pageSize, setPageSize] = useState(25);
  const [page, setPage] = useState(1);
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
        const runtimeEntries = group.runtimes.map((runtime) => {
          const finitePrivateGrant = finitePrivateGrantSummaryForRuntime(runtime, finitePrivate.state);
          return {
            runtime,
            finitePrivateGrant,
            cells: adminUserTableCells(runtime, account, finitePrivateGrant, finitePrivate.state?.profiles, finitePrivate.state),
            auditEvents: adminUserAuditEvents(runtime, finitePrivateGrant, finitePrivate.state),
          };
        });
        return {
          group,
          account,
          runtimeEntries,
        };
      }),
    [finitePrivate.state, result.runtimes]
  );
  const filteredUserEntries = useMemo(
    () =>
      userEntries
        .map(({ group, account, runtimeEntries }) => ({
          group,
          runtimeEntries: runtimeEntries.filter(
            ({ runtime, finitePrivateGrant, cells, auditEvents }) =>
              adminRuntimeMatchesSearch(
                runtime,
                account,
                query,
                finitePrivateGrant,
                [...cells.map((cell) => cell.value), ...auditEvents.map((event) => [event.id, event.action, event.actor, event.target_type, event.target_id, event.grant_id, event.api_key_id, event.created_at, JSON.stringify(event.metadata)].join(" "))],
              ),
          ),
          account,
        }))
        .filter(({ runtimeEntries }) => runtimeEntries.length > 0),
    [query, userEntries]
  );
  const tableRows = useMemo(() =>
    filteredUserEntries.flatMap(({ account, runtimeEntries }) =>
      runtimeEntries.map((entry) => ({ account, entry }))
    ).sort((a, b) => compareAdminTableCells(
      a.entry.cells.find((cell) => cell.label === sort.column),
      b.entry.cells.find((cell) => cell.label === sort.column),
      sort.direction,
    )), [filteredUserEntries, sort]);

  const pageCount = Math.max(1, Math.ceil(tableRows.length / pageSize));
  const currentPage = Math.min(page, pageCount);
  const pageStart = (currentPage - 1) * pageSize;
  const paginatedRows = tableRows.slice(pageStart, pageStart + pageSize);

  return (
    <section className={styles.panel}>
      <Dialog open={selectedAgent !== null} onOpenChange={(open) => { if (!open) setSelectedAgent(null); }}>
        <DialogContent className={`${dialogStyles.dialog} max-h-[90dvh] gap-0 overflow-y-auto p-0 sm:max-w-[1080px]`}
          onCloseAutoFocus={(event) => { event.preventDefault(); selectedRowRef.current?.focus({ preventScroll: true }); }}>
          {selectedAgent && <AdminAgentDetails entry={selectedAgent.entry} account={selectedAgent.account} profiles={finitePrivate.state?.profiles ?? []} />}
        </DialogContent>
      </Dialog>
      {finitePrivate.error && <p role="status" className="text-sm text-muted-foreground">Finite Private details unavailable: {finitePrivate.error}</p>}
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
          <AdminUsersToolbar>
          <div className="flex flex-wrap items-center gap-3">
            <div className="w-80 max-w-full">
              <div className="relative">
                <SearchIcon
                  className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
                  aria-hidden
                />
                <Input
                  id="adminAgentFilter"
                  aria-label="Filter agents"
                  value={query}
                  onChange={(event) => { setQuery(event.target.value); setPage(1); }}
                  className="h-10 rounded-full pl-8"
                  placeholder="Search all agent details…"
                  type="search"
                />
              </div>
            </div>
          </div>
          </AdminUsersToolbar>

          {filteredUserEntries.length === 0 ? (
            <div className="ocean-empty-state">No agents match that filter.</div>
          ) : (
            <>
            <AdminTableScroll sort={sort} onSort={(column) => { setPage(1); setSort((previous) => ({
              column,
              direction: previous.column === column && previous.direction === "ascending" ? "descending" : "ascending",
            })); }}>
              <table className={styles.table}>
                <caption className="sr-only">Users and agents with Finite Private usage and runtime details</caption>
                <thead><tr>
                  {filteredUserEntries[0].runtimeEntries[0].cells.map((cell) => (
                    <th key={cell.label} scope="col" aria-sort={sort.column === cell.label ? sort.direction : "none"}><span className={styles.sortLabel}>{cell.label}</span></th>
                  ))}
                </tr></thead>
                <tbody>
                  {paginatedRows.map(({ account, entry }) => {
                    const id = entry.runtime.agent_runtime_id;
                    const cells = entry.cells;
                    return (
                      <tr key={id} tabIndex={0} aria-label={`Open controls for ${entry.runtime.project_display_name}`}
                        aria-haspopup="dialog"
                        onClick={(event) => {
                          if (window.getSelection()?.toString()) return;
                          selectedRowRef.current = event.currentTarget;
                          setSelectedAgent({ entry, account });
                        }}
                        onKeyDown={(event) => {
                          if (event.key !== "Enter" && event.key !== " ") return;
                          event.preventDefault();
                          selectedRowRef.current = event.currentTarget;
                          setSelectedAgent({ entry, account });
                        }}>
                        {cells.map((cell) => (
                          <td key={cell.label} className={cell.numeric ? styles.numeric : undefined}>
                            {cell.usage ? (
                              <FinitePrivateUsageProgress
                                usedUnits={cell.usage.usedUnits}
                                limitUnits={cell.usage.limitUnits}
                                stacked
                                className="min-w-72 text-left"
                              />
                            ) : cell.value}
                          </td>
                        ))}
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </AdminTableScroll>
            <nav aria-label="Agent table pagination" className="flex flex-wrap items-center justify-between gap-4 py-3 text-sm">
              <p className="text-muted-foreground" role="status">
                {pageStart + 1}–{Math.min(pageStart + pageSize, tableRows.length)} of {tableRows.length} agents
              </p>
              <div className="flex flex-wrap items-center gap-4">
                <div className="flex items-center gap-2">
                  <span id="agentsPerPageLabel" className="text-muted-foreground">Agents per page</span>
                  <Select value={String(pageSize)} onValueChange={(value) => { setPageSize(Number(value)); setPage(1); }}>
                    <SelectTrigger aria-labelledby="agentsPerPageLabel" size="sm"><SelectValue /></SelectTrigger>
                    <SelectContent>{[10, 25, 50, 100].map((size) => <SelectItem key={size} value={String(size)}>{size}</SelectItem>)}</SelectContent>
                  </Select>
                </div>
                <div className="flex items-center gap-2">
                  <span id="agentsPageLabel" className="text-muted-foreground">Page</span>
                  <Select value={String(currentPage)} onValueChange={(value) => setPage(Number(value))}>
                    <SelectTrigger aria-labelledby="agentsPageLabel" size="sm"><SelectValue /></SelectTrigger>
                    <SelectContent>{Array.from({ length: pageCount }, (_, index) => <SelectItem key={index + 1} value={String(index + 1)}>{index + 1}</SelectItem>)}</SelectContent>
                  </Select>
                  <span className="text-muted-foreground">of {pageCount}</span>
                </div>
                <div className="flex gap-1">
                  <Button type="button" variant="outline" size="sm" disabled={currentPage === 1} onClick={() => setPage(currentPage - 1)}>Previous</Button>
                  <Button type="button" variant="outline" size="sm" disabled={currentPage === pageCount} onClick={() => setPage(currentPage + 1)}>Next</Button>
                </div>
              </div>
            </nav>
            </>
          )}
        </div>
      )}
    </section>
  );
}

function AdminAgentDetails({ entry, account, profiles }: {
  entry: ProvisionedRuntimeEntry;
  account: CoreFinitePrivateAdminAccount | null;
  profiles: CoreFinitePrivateLimitProfile[];
}) {
  const { runtime, finitePrivateGrant } = entry;
  const grantId = finitePrivateGrant?.grantId ?? account?.grant.id;
  const profileId = finitePrivateGrant?.limitProfileId ?? account?.grant.limit_profile_id;
  const usedUnits = finitePrivateGrant?.currentWindowUsedUnits ?? account?.grant.current_window_used_units;
  const profile = profiles.find((candidate) => candidate.id === profileId);
  const assignable = finitePrivateAssignableProfiles(profiles);
  const facts = [
    ["Host / machine", `${runtime.source_host_id} / ${runtime.source_machine_id}`],
    ["Runtime version", runtime.runtime_artifact_version_label ?? runtime.runtime_artifact_id],
    ["Last heartbeat", runtime.last_heartbeat_at ? formatAdminDate(runtime.last_heartbeat_at) : "Never reported"],
    ["Runtime ID", runtime.agent_runtime_id],
    ["Project ID", runtime.project_id],
    ["Grant ID", grantId],
    ["Health report", runtimeHealthLabel(runtime)],
  ];
  return (
    <>
      <header className={dialogStyles.header}>
        <DialogTitle className={dialogStyles.title}>{runtime.project_display_name}</DialogTitle>
        <DialogDescription className="mt-2">Owner: {runtime.owner_email ?? account?.email ?? "Unknown account"}</DialogDescription>
        <div className={dialogStyles.status}>
          <span><ActivityIcon className="size-3.5" aria-hidden="true" />{runtime.runtime_status}</span>
          <span><ShieldCheckIcon className="size-3.5" aria-hidden="true" />Health: {runtime.runtime_health?.status ?? "Unknown"}</span>
          <span>{runtime.active_finite_private_key_count} active {runtime.active_finite_private_key_count === 1 ? "key" : "keys"}</span>
        </div>
      </header>
      <div className={dialogStyles.columns}>
        <div className={`${dialogStyles.main} ${formStyles.surface}`}>
          <section className={dialogStyles.section}>
            <h3>Usage & limits</h3>
            <p className={dialogStyles.description}>Current burst window · shared by agents on this grant.</p>
            <div className={dialogStyles.usageRow}>
              {grantId && profileId && assignable.length > 0 ? (
                <AdminFinitePrivateProfileForm grantId={grantId} currentProfileId={profileId} profiles={assignable} />
              ) : null}
              <div className={dialogStyles.usage}>
                {usedUnits != null && profile ? (
                  <FinitePrivateUsageProgress usedUnits={usedUnits} limitUnits={profile.burst_limit_units} />
                ) : <p className="text-sm text-muted-foreground">{usedUnits == null ? "Usage is unavailable." : `${formatWeightedTokens(usedUnits)} weighted tokens used. Limit details are unavailable.`}</p>}
              </div>
            </div>
          </section>
          <section className={dialogStyles.section}>
            <h3>Actions</h3>
            <p className={dialogStyles.description}>Restart this agent, recover its chat runtime, or reset usage for its grant.</p>
            <RuntimeActions runtime={runtime} finitePrivateAccount={account} finitePrivateGrant={finitePrivateGrant} />
          </section>
          <section className={dialogStyles.section}>
            <h3>Account keys</h3>
            <p className={dialogStyles.description}>Manage access for {account?.email ?? runtime.owner_email ?? "this account"}. Revoking a key stops requests that use it.</p>
            {account?.apiKeys.length ? account.apiKeys.map((apiKey) => (
              <FinitePrivateAccountKey key={apiKey.id} apiKey={apiKey} account={account} plain />
            )) : <p className="text-sm text-muted-foreground">No account key details available.</p>}
          </section>
          <section className={dialogStyles.section}>
            <details className={dialogStyles.moreDetails}>
              <summary><ChevronRightIcon aria-hidden="true" />All agent & account details</summary>
              <dl className={dialogStyles.allFacts}>
                {entry.cells.map((cell) => <div key={cell.label}><dt>{cell.label}</dt><dd>{cell.value}</dd></div>)}
              </dl>
            </details>
          </section>
          <section className={dialogStyles.section}>
            <details className={dialogStyles.moreDetails}>
              <summary><ChevronRightIcon aria-hidden="true" />Related audit activity ({entry.auditEvents.length})</summary>
              <p className="mt-3 text-xs text-muted-foreground">Events returned for this runtime, project, and its keys or shared grants.</p>
              {entry.auditEvents.length ? entry.auditEvents.map((event) => (
                <div key={event.id} className="grid gap-1 border-b border-border py-3 text-xs">
                  <strong>{event.action}</strong>
                  <span>{event.created_at} · {event.actor}</span>
                  <span className="break-all">{event.target_type}: {event.target_id}</span>
                  <span className="break-all">Event: {event.id} · Grant: {event.grant_id ?? "None"} · Key: {event.api_key_id ?? "None"}</span>
                  <details className="mt-2"><summary className="cursor-pointer">Event metadata</summary><pre className="mt-2 whitespace-pre-wrap break-all">{JSON.stringify(event.metadata, null, 2)}</pre></details>
                </div>
              )) : <p className="mt-3 text-sm text-muted-foreground">No related events returned.</p>}
            </details>
          </section>
        </div>
        <aside className={dialogStyles.aside}>
          <h3>Runtime details</h3>
          <p className={dialogStyles.description}>Identifiers and the latest reported state.</p>
          <dl className={dialogStyles.facts}>
            {facts.map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value ?? "Unavailable"}</dd></div>)}
          </dl>
          {runtime.published_app_urls.length > 0 && <div className="mt-6 grid gap-2 text-xs">
            <h3>Published sites</h3>
            {runtime.published_app_urls.map((url) => <a key={url} href={url} target="_blank" rel="noreferrer" className="break-all underline">{url}</a>)}
          </div>}
        </aside>
      </div>
    </>
  );
}

function RuntimeActions({ runtime, finitePrivateAccount, finitePrivateGrant }: {
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
    ? `Reset burst usage for ${runtime.project_display_name} (${runtime.source_machine_id}) grant ${finitePrivateGrant.grantId}? Current usage is ${formatWeightedTokens(finitePrivateGrant.currentWindowUsedUnits)} weighted tokens.`
    : finitePrivateAccount
      ? `Reset Finite Private usage for ${runtime.project_display_name} (${finitePrivateAccount.email})?`
      : "";

  return (
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
  );
}

function FinitePrivateAccountKey({
  apiKey,
  account,
  plain = false,
}: {
  apiKey: CoreFinitePrivateApiKey;
  account: CoreFinitePrivateAdminAccount;
  plain?: boolean;
}) {
  const project = account.projects.find((candidate) => candidate.id === apiKey.project_id);
  return (
    <div className={plain ? "flex flex-wrap items-start justify-between gap-3 border-t border-border py-4" : "flex flex-wrap items-start justify-between gap-3 rounded-[var(--radius-card-inner)] border border-border bg-black/10 p-3"}>
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

/**
 * The evidence behind Core's derived status pill, shown only when Core sent
 * it: the latest health report's state and when the runner observed it
 * (absolute, like every other admin timestamp), plus the raw lifecycle latch
 * whenever it differs.
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
