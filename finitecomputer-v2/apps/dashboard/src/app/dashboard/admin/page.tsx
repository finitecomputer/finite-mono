import Link from "next/link";
import { notFound } from "next/navigation";
import {
  ActivityIcon,
  BanIcon,
  ExternalLinkIcon,
  KeyRoundIcon,
  RotateCcwIcon,
  ServerIcon,
  ShieldCheckIcon,
  WrenchIcon,
} from "lucide-react";

import {
  adminOpsRecoverRuntimeAction,
  adminOpsRevokeLaunchCodeBatchAction,
  adminOpsRevokeFinitePrivateKeyAction,
  adminOpsResetFinitePrivateWindowAction,
  adminOpsRestartRuntimeAction,
} from "@/app/actions";
import {
  AdminFinitePrivateProfileForm,
  AdminFriendKeyIssueForm,
  AdminLaunchCodeBatchIssueForm,
  AdminRotateKeyForm,
  ConfirmSubmitButton,
} from "@/components/admin-ops-forms";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  canAccessAdminOps,
  finitePrivateAccountForProject,
  finitePrivateAssignableProfiles,
  finitePrivateProfileLabel,
  groupAdminRuntimesByOwner,
  heartbeatAgeLabel,
  launchCodeHostingTierLabel,
} from "@/lib/admin-ops";
import {
  coreAdminRuntimeSupportsRecovery,
  coreAdminRuntimeSupportsRestart,
  coreAdminRuntimeSupportsUpgrade,
  loadCoreAdminRuntimes,
  loadCoreFinitePrivateAdminState,
  loadCoreLaunchCodeBatches,
  type CoreAdminRuntimeOverview,
  type CoreAdminRuntimesResult,
  type CoreFinitePrivateAdminStateResult,
  type CoreFinitePrivateAdminAccount,
  type CoreFinitePrivateApiKey,
  type CoreFinitePrivateLimitProfile,
  type CoreLaunchCodeBatchDetails,
  type CoreLaunchCodeBatchesResult,
  type CoreRuntimeStatus,
} from "@/lib/core-client";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";

export default async function AdminOpsPage() {
  const viewer = await loadOptionalViewerContext();
  if (!canAccessAdminOps(viewer)) {
    notFound();
  }

  const [runtimes, finitePrivate, launchCodeBatches] = await Promise.all([
    loadCoreAdminRuntimes(),
    loadCoreFinitePrivateAdminState(),
    loadCoreLaunchCodeBatches(),
  ]);

  return (
    <div className="ocean-page-stack">
      <section className="ocean-page-hero">
        <div className="ocean-page-hero__main">
          <span className="ocean-page-hero__icon" aria-hidden>
            <WrenchIcon className="size-5" />
          </span>
          <div>
            <h1 className="ocean-page-hero__title">Admin Ops</h1>
            <p className="ocean-page-hero__description">
              Provisioned boxes and Finite Private management. Core authorizes
              every action against its own admin allowlist.
            </p>
          </div>
        </div>
      </section>

      <Tabs defaultValue="users" className="gap-4">
        <TabsList aria-label="Admin sections">
          <TabsTrigger value="users">Users</TabsTrigger>
          <TabsTrigger value="invites">Invites</TabsTrigger>
          <TabsTrigger value="finite-private">Finite Private</TabsTrigger>
        </TabsList>
        <TabsContent value="users">
          <ProvisionedBoxesPanel result={runtimes} finitePrivate={finitePrivate} />
        </TabsContent>
        <TabsContent value="invites">
          <LaunchCodeBatchesPanel result={launchCodeBatches} />
        </TabsContent>
        <TabsContent value="finite-private">
          <FinitePrivateOpsPanel result={finitePrivate} />
        </TabsContent>
      </Tabs>
    </div>
  );
}

function LaunchCodeBatchesPanel({ result }: { result: CoreLaunchCodeBatchesResult }) {
  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          <KeyRoundIcon className="size-5" />
        </span>
        <div>
          <h2 className="ocean-utility-card__title">Launch Codes</h2>
          <p className="text-sm text-muted-foreground">
            Issue bounded sponsored access for an approved canary or training cohort. Plaintext codes appear only once.
          </p>
        </div>
      </div>

      {!result.configured ? (
        <div className="ocean-empty-state">Finite Core is not configured: {result.missing.join(", ")}.</div>
      ) : result.error ? (
        <div className="ocean-empty-state">{result.error}</div>
      ) : (
        <div className="grid gap-4">
          <AdminLaunchCodeBatchIssueForm />
          <LaunchCodeBatchList batches={result.batches ?? []} />
        </div>
      )}
    </section>
  );
}

function LaunchCodeBatchList({ batches }: { batches: CoreLaunchCodeBatchDetails[] }) {
  if (batches.length === 0) {
    return <div className="ocean-empty-state">No Launch Code batches yet.</div>;
  }
  return (
    <div className="grid gap-3">
      <div className="text-sm font-semibold text-foreground">Issued batches</div>
      {batches.map(({ batch, codes }) => {
        const redeemed = codes.filter((code) => Boolean(code.redeemed_at)).length;
        const revoked = Boolean(batch.revoked_at);
        return (
          <div key={batch.id} className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4 md:grid-cols-[minmax(0,1fr)_auto]">
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <span className="truncate font-semibold text-foreground">{batch.name}</span>
                <span className="rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">
                  {revoked ? "revoked" : "active"}
                </span>
                <span className="rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">
                  {launchCodeHostingTierLabel(batch.hosting_tier)}
                </span>
              </div>
              <div className="mt-1 grid gap-1 text-xs text-muted-foreground">
                <span>{batch.code_count} codes · {redeemed} redeemed</span>
                <span>expires {formatAdminDate(batch.expires_at)}</span>
                <span>created {formatAdminDate(batch.created_at)}</span>
                {batch.revoked_at ? <span>revoked {formatAdminDate(batch.revoked_at)}</span> : null}
              </div>
              <details className="mt-3 text-xs text-muted-foreground">
                <summary className="cursor-pointer">
                  {launchCodeHostingTierLabel(batch.hosting_tier)} batch details
                </summary>
                <div className="mt-2 grid gap-1 font-mono">
                  {codes.map((code) => (
                    <span key={code.id}>
                      {code.id} · {code.redeemed_customer_org_id ?? "unredeemed"}
                      {code.redeemed_at ? ` · ${formatAdminDate(code.redeemed_at)}` : ""}
                    </span>
                  ))}
                </div>
              </details>
            </div>
            {!revoked ? (
              <form action={adminOpsRevokeLaunchCodeBatchAction}>
                <input type="hidden" name="batchId" value={batch.id} />
                <ConfirmSubmitButton
                  variant="outline"
                  size="sm"
                  pendingLabel="Revoking..."
                  confirmMessage={`Revoke ${batch.name}? Unredeemed Launch Codes in this batch will stop working.`}
                >
                  <BanIcon />
                  Revoke batch
                </ConfirmSubmitButton>
              </form>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

function formatAdminDate(value: string) {
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? new Date(timestamp).toLocaleString() : value;
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

function heartbeatLabel(lastHeartbeatAt: string | null | undefined) {
  return heartbeatAgeLabel(lastHeartbeatAt, Date.now());
}

function ProvisionedBoxesPanel({
  result,
  finitePrivate,
}: {
  result: CoreAdminRuntimesResult;
  finitePrivate: CoreFinitePrivateAdminStateResult;
}) {
  const userGroups = groupAdminRuntimesByOwner(result.runtimes ?? []);
  const profiles = finitePrivateAssignableProfiles(finitePrivate.state?.profiles);
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
      ) : userGroups.length === 0 ? (
        <div className="ocean-empty-state">No users with provisioned agents yet.</div>
      ) : (
        <div className="grid gap-3">
          {userGroups.map((group) => {
            const account = group.runtimes
              .map((runtime) =>
                finitePrivateAccountForProject(
                  finitePrivate.state?.accounts,
                  runtime.project_id
                )
              )
              .find((candidate) => candidate !== null) ?? null;
            return (
              <ProvisionedUserCard
                key={group.key}
                email={group.email}
                runtimes={group.runtimes}
                finitePrivateAccount={account}
                finitePrivateDetailsAvailable={
                  finitePrivate.state?.accounts !== undefined
                }
                profiles={profiles}
              />
            );
          })}
        </div>
      )}
    </section>
  );
}

function ProvisionedUserCard({
  email,
  runtimes,
  finitePrivateAccount,
  finitePrivateDetailsAvailable,
  profiles,
}: {
  email: string | null;
  runtimes: CoreAdminRuntimeOverview[];
  finitePrivateAccount: CoreFinitePrivateAdminAccount | null;
  finitePrivateDetailsAvailable: boolean;
  profiles: CoreFinitePrivateLimitProfile[];
}) {
  return (
    <article className="grid gap-4 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h3 className="font-semibold text-foreground">{email ?? "Unknown account"}</h3>
          <p className="text-xs text-muted-foreground">
            {runtimes.length} agent{runtimes.length === 1 ? "" : "s"}
          </p>
        </div>
        {finitePrivateAccount ? (
          <span className="rounded-full border border-emerald-400/40 px-2 py-0.5 text-xs text-emerald-400">
            Finite Private {finitePrivateAccount.grant.status}
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
        {runtimes.map((runtime) => (
          <ProvisionedRuntimeRow key={runtime.agent_runtime_id} runtime={runtime} />
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

function ProvisionedRuntimeRow({ runtime }: { runtime: CoreAdminRuntimeOverview }) {
  const canRestart = coreAdminRuntimeSupportsRestart(runtime);
  const canRecover = coreAdminRuntimeSupportsRecovery(runtime);
  const canUpgrade = coreAdminRuntimeSupportsUpgrade(runtime);

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
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <div className="flex flex-wrap items-center gap-2 font-semibold text-foreground">
            <ShieldCheckIcon className="size-4" />
            Finite Private
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            {finitePrivateProfileLabel(grant.limit_profile_id)} · {grant.current_window_used_units.toLocaleString()} units used
          </p>
        </div>
        <form action={adminOpsResetFinitePrivateWindowAction}>
          <input type="hidden" name="grantId" value={grant.id} />
          <ConfirmSubmitButton
            variant="outline"
            size="sm"
            pendingLabel="Resetting..."
            confirmMessage={`Reset Finite Private usage for ${account.email}?`}
          >
            <RotateCcwIcon />
            Reset usage
          </ConfirmSubmitButton>
        </form>
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

function FinitePrivateOpsPanel({
  result,
}: {
  result: CoreFinitePrivateAdminStateResult;
}) {
  const state = result.state;
  const activeGrantCount =
    state?.grants.filter((grant) => grant.status === "active").length ?? 0;
  const activeKeyCount =
    state?.apiKeys.filter((key) => key.status === "active").length ?? 0;
  const usedUnits =
    state?.grants.reduce((total, grant) => total + grant.current_window_used_units, 0) ?? 0;
  const profiles = finitePrivateAssignableProfiles(state?.profiles);

  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          <ShieldCheckIcon className="size-5" />
        </span>
        <div>
          <h2 className="ocean-utility-card__title">Finite Private</h2>
          <p className="text-sm text-muted-foreground">
            Mint standalone friends-and-family keys for testing. Account grant,
            usage, profile, and key controls live on each card in Users.
          </p>
        </div>
      </div>

      {!result.configured ? (
        <div className="ocean-empty-state">
          Finite Core is not configured: {result.missing.join(", ")}.
        </div>
      ) : result.error ? (
        <div className="ocean-empty-state">{result.error}</div>
      ) : state ? (
        <div className="grid gap-4">
          <div className="ocean-metric-grid">
            <div className="ocean-metric">
              <span>{activeGrantCount}</span>
              <small>Active grants</small>
            </div>
            <div className="ocean-metric">
              <span>{activeKeyCount}</span>
              <small>Active keys</small>
            </div>
            <div className="ocean-metric">
              <span>{usedUnits}</span>
              <small>Burst window units used</small>
            </div>
          </div>

          <AdminFriendKeyIssueForm profiles={profiles} />
        </div>
      ) : null}
    </section>
  );
}
