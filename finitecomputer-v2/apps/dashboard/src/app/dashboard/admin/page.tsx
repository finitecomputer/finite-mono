import { notFound } from "next/navigation";
import {
  BanIcon,
  KeyRoundIcon,
  ShieldCheckIcon,
  WrenchIcon,
} from "lucide-react";

import { adminOpsRevokeLaunchCodeBatchAction } from "@/app/actions";
import {
  AdminFriendKeyIssueForm,
  AdminLaunchCodeBatchIssueForm,
  ConfirmSubmitButton,
} from "@/components/admin-ops-forms";
import { AdminUsersPanel } from "@/components/admin-users-panel";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  canAccessAdminOps,
  finitePrivateAssignableProfiles,
  launchCodeHostingTierLabel,
} from "@/lib/admin-ops";
import {
  loadCoreAdminRuntimes,
  loadCoreFinitePrivateAdminState,
  loadCoreLaunchCodeBatches,
  type CoreFinitePrivateAdminStateResult,
  type CoreLaunchCodeBatchDetails,
  type CoreLaunchCodeBatchesResult,
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
          <AdminUsersPanel result={runtimes} finitePrivate={finitePrivate} />
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
