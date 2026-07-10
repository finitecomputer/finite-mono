import Link from "next/link";
import { redirect } from "next/navigation";
import { cookies } from "next/headers";
import { randomUUID } from "node:crypto";
import {
  ActivityIcon,
  BanIcon,
  CheckCircle2Icon,
  CreditCardIcon,
  KeyRoundIcon,
  Loader2Icon,
  PlusIcon,
  RotateCcwIcon,
  ShieldCheckIcon,
  ServerIcon,
} from "lucide-react";

import {
  approveFinitePrivateGrantAction,
  cancelFailedAgentCreationRequestAction,
  claimCoreImportCandidatesAction,
  issueFinitePrivateApiKeyAction,
  resetFinitePrivateGrantAction,
  revokeFinitePrivateApiKeyAction,
  revokeFinitePrivateGrantAction,
  rotateFinitePrivateApiKeyAction,
} from "@/app/actions";
import { CoreAgentCreationForm } from "@/components/core-agent-creation-form";
import { FormActionButton } from "@/components/form-action-button";
import { PendingRefresh } from "@/components/pending-refresh";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  coreAgentCreationRequestForProject,
  coreProjectLocationLabel,
  coreProjectLaunchStatusLabel,
  coreProjectLabel,
  loadCoreFinitePrivateAdminState,
  loadCoreBillingOverview,
  loadCoreMe,
  type CoreAgentCreationRequestSummary,
  type CoreFinitePrivateAdminStateResult,
  type CoreFinitePrivateApiKey,
  type CoreFinitePrivateGrant,
  type CoreMeResult,
  type CoreProjectImportCandidate,
  type CoreVisibleProject,
} from "@/lib/core-client";
import {
  BILLING_SYNC_MAX_POLL_INTERVAL_MS,
  BILLING_SYNC_POLL_INTERVAL_MS,
  billingSyncStampRedirectPath,
  parseBillingReturnParam,
  parseBillingSyncStartedAt,
  resolveBillingReturnStateNow,
} from "@/lib/billing-return";
import { coreProjectOverviewHref } from "@/lib/dashboard-machine-access";
import {
  dashboardDevLaunchCode,
  getAccountAuthContext,
  loadOptionalViewerContext,
} from "@/lib/dashboard-auth";
import { stripeBillingStatus } from "@/lib/stripe-billing";
import {
  AGENT_DRAFT_COOKIE,
  defaultRunnerClass,
  unsealAgentOnboardingDraft,
  type AgentOnboardingDraft,
} from "@/lib/agent-onboarding";

function shortValue(value: string | null | undefined, length = 12) {
  if (!value) {
    return "unknown";
  }

  return value.length > length ? value.slice(0, length) : value;
}

type DashboardSearchParams = {
  agentCreationError?: string | string[];
  billing?: string | string[];
  billingSyncStartedAt?: string | string[];
  creation?: string | string[];
  new?: string | string[];
};

function firstSearchParam(value: string | string[] | undefined) {
  if (Array.isArray(value)) {
    return value[0] ?? null;
  }
  return value ?? null;
}

export default async function DashboardPage({
  searchParams,
}: {
  searchParams: Promise<DashboardSearchParams>;
}) {
  const query = await searchParams;
  const agentCreationError = firstSearchParam(query.agentCreationError);
  const billingReturnParam = parseBillingReturnParam(firstSearchParam(query.billing));
  const billingSyncStartedAtMs = parseBillingSyncStartedAt(
    firstSearchParam(query.billingSyncStartedAt)
  );
  const isNewAgentFlow = firstSearchParam(query.new) === "1";
  const trackedCreationRequestId = firstSearchParam(query.creation)?.trim() || null;
  const [viewer, account] = await Promise.all([
    loadOptionalViewerContext(),
    getAccountAuthContext(),
  ]);

  const core = await loadCoreMe();
  if (core.configured || !viewer.isAdmin) {
    // The checkout-return sync poll must observe Core directly, not a cached
    // read, so the webhook-arrival flip is seen as soon as it lands.
    const billing = await loadCoreBillingOverview({
      cacheMode: billingReturnParam === "success" ? "fresh" : "swr",
    });
    const draft = await unsealAgentOnboardingDraft(
      (await cookies()).get(AGENT_DRAFT_COOKIE)?.value,
      account.workosUserId
    );
    const coreProjects = core.me?.projects ?? [];
    const agentCreationRequests = core.me?.agent_creation_requests ?? [];
    const claimableCandidates = core.me?.claimable_candidates ?? [];
    const requestedAgentCreationRequests = agentCreationRequests.filter(
      (request) => request.status === "requested" || request.status === "launching"
    );
    const failedAgentCreationRequests = agentCreationRequests.filter(
      (request) => request.status === "failed"
    );
    // Imports are intentionally hidden from the finite.computer self-serve surface for Oslo.
    const showImportCandidates = false;
    const firstAgentHref = coreProjects
      .map((project) => coreProjectOverviewHref(project))
      .find((href): href is string => Boolean(href));
    const trackedCreationRequest = trackedCreationRequestId
      ? agentCreationRequests.find((request) => request.id === trackedCreationRequestId) ?? null
      : null;
    const trackedProjectHref = trackedCreationRequest
      ? coreProjects
          .filter((project) => project.project.id === trackedCreationRequest.project_id)
          .map((project) => coreProjectOverviewHref(project))
          .find((href): href is string => Boolean(href)) ?? null
      : null;

    if (firstAgentHref && !isNewAgentFlow) {
      redirect(firstAgentHref);
    }
    if (
      isNewAgentFlow &&
      trackedCreationRequest?.status === "running" &&
      trackedProjectHref
    ) {
      redirect(trackedProjectHref);
    }

    const pendingAgentCreationRequests =
      trackedCreationRequest?.status === "running" && !trackedProjectHref
        ? [...requestedAgentCreationRequests, trackedCreationRequest]
        : requestedAgentCreationRequests;
    const hasPendingAgentCreation = pendingAgentCreationRequests.length > 0;

    const billingReturn = resolveBillingReturnStateNow({
      billingParam: billingReturnParam,
      billingLoaded: Boolean(billing.billing),
      requiresBilling: Boolean(billing.billing?.requires_billing),
      syncStartedAtMs: billingSyncStartedAtMs,
    });
    if (billingReturn.kind === "stamp-sync-start") {
      // First render after a successful checkout while Core still waits on
      // the webhook: stamp the sync window start so the poll stays bounded.
      redirect(billingSyncStampRedirectPath(undefined, { newAgent: isNewAgentFlow }));
    }
    const billingSyncPending =
      billingReturn.kind === "confirming" || billingReturn.kind === "sync-timeout";
    const localDevelopmentLaunchCode = dashboardDevLaunchCode(account);

    if (draft && billing.billing?.can_create_agent) {
      redirect("/dashboard/agent-creation-requests/complete");
    }

    const showCreateAgent =
      core.configured &&
      Boolean(core.account.email) &&
      (coreProjects.length === 0 || isNewAgentFlow) &&
      pendingAgentCreationRequests.length === 0 &&
      !billingSyncPending &&
      failedAgentCreationRequests.length === 0;
    // While a successful checkout is still syncing, the billing setup panel
    // (and its Start checkout button) must stay hidden to avoid a second
    // subscription attempt.
    const showBillingSyncState =
      billingSyncPending &&
      core.configured &&
      Boolean(core.account.email) &&
      !showCreateAgent &&
      (coreProjects.length === 0 || isNewAgentFlow) &&
      pendingAgentCreationRequests.length === 0 &&
      failedAgentCreationRequests.length === 0;
    const showEmptyAccount =
      coreProjects.length === 0 &&
      pendingAgentCreationRequests.length === 0 &&
      failedAgentCreationRequests.length === 0 &&
      !showCreateAgent &&
      !showBillingSyncState;

    return (
      <div className="ocean-page-stack">
        <PendingRefresh enabled={hasPendingAgentCreation} />
        {coreProjects.length > 0 && !isNewAgentFlow ? (
          <CoreProjectsPanel
            projects={coreProjects}
            agentCreationRequests={agentCreationRequests}
          />
        ) : null}
        {showImportCandidates && claimableCandidates.length ? (
          <CoreImportCandidatesPanel candidates={claimableCandidates} />
        ) : null}
        {pendingAgentCreationRequests.length ? (
          <CoreAgentCreationStatusPanel requests={pendingAgentCreationRequests} />
        ) : null}
        {failedAgentCreationRequests.length ? (
          <CoreAgentCreationFailedPanel requests={failedAgentCreationRequests} />
        ) : null}
        {showBillingSyncState && billingReturn.kind === "confirming" ? (
          <BillingSyncWaitPanel deadlineAtMs={billingReturn.deadlineAtMs} />
        ) : null}
        {showBillingSyncState && billingReturn.kind === "sync-timeout" ? (
          <BillingSyncTimeoutPanel />
        ) : null}
        {showCreateAgent && billingReturn.kind === "cancelled" ? (
          <BillingCheckoutCancelledNotice />
        ) : null}
        {showCreateAgent ? (
          <CoreAgentCreationPanel
            error={agentCreationError}
            draft={draft}
            requiresAccess={
              !billing.billing?.can_create_agent && !localDevelopmentLaunchCode
            }
          />
        ) : null}
        {showEmptyAccount ? (
          <section className="ocean-utility-card">
            <div className="ocean-utility-card__header">
              <span className="ocean-utility-card__icon" aria-hidden>
                <ServerIcon className="size-5" />
              </span>
              <div>
                <h1 className="ocean-utility-card__title">No agent yet</h1>
                <p className="text-sm text-muted-foreground">
                  {emptyAccountMessage(core)}
                </p>
              </div>
            </div>
          </section>
        ) : null}
      </div>
    );
  }

  const deploySourceRev = viewer.deployMetadata?.source_rev ?? null;
  const finitePrivateAdmin = await loadCoreFinitePrivateAdminState({ cacheMode: "swr" });

  return (
    <div className="ocean-page-stack">
      <section className="ocean-page-hero">
        <div className="ocean-page-hero__main">
          <span className="ocean-page-hero__icon" aria-hidden>
            <ServerIcon className="size-5" />
          </span>
          <div>
            <h1 className="ocean-page-hero__title">Admin</h1>
            <p className="ocean-page-hero__description">
              Inspect Core state and administer Finite Private.
            </p>
          </div>
        </div>
        <div className="ocean-metric-grid">
          <div className="ocean-metric">
            <span>{shortValue(deploySourceRev, 7)}</span>
            <small>Deploy commit</small>
          </div>
        </div>
        <Button asChild variant="outline" size="sm">
          <Link href="/dashboard/admin">
            <ServerIcon />
            Admin Ops
          </Link>
        </Button>
      </section>

      <FinitePrivateAdminPanel result={finitePrivateAdmin} />
    </div>
  );
}

function FinitePrivateAdminPanel({
  result,
}: {
  result: CoreFinitePrivateAdminStateResult;
}) {
  const state = result.state;
  const activeGrantCount = state?.grants.filter((grant) => grant.status === "active").length ?? 0;
  const activeKeyCount = state?.apiKeys.filter((key) => key.status === "active").length ?? 0;
  const usedUnits =
    state?.grants.reduce((total, grant) => total + grant.current_window_used_units, 0) ?? 0;
  const latestEvents = state?.adminAuditEvents.slice(-6).reverse() ?? [];

  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          <ShieldCheckIcon className="size-5" />
        </span>
        <div>
          <h2 className="ocean-utility-card__title">Finite Private</h2>
          <p className="text-sm text-muted-foreground">
            Grant access, rotate keys, reset usage, and inspect current status.
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
              <small>Used units</small>
            </div>
          </div>

          <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
            <form
              action={approveFinitePrivateGrantAction}
              className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4"
            >
              <div className="flex items-center gap-2 font-semibold text-foreground">
                <ShieldCheckIcon className="size-4" />
                Approve grant
              </div>
              <div className="grid gap-2">
                <Label htmlFor="fpVerifiedEmail">Verified email</Label>
                <Input id="fpVerifiedEmail" name="verifiedEmail" type="email" required />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="fpWorkosUserId">WorkOS user id</Label>
                <Input id="fpWorkosUserId" name="workosUserId" />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="fpLimitProfileId">Limit profile</Label>
                <Input
                  id="fpLimitProfileId"
                  name="limitProfileId"
                  defaultValue="finite-private-generous"
                />
              </div>
              <FormActionButton className="w-fit" pendingLabel="Approving...">
                <ShieldCheckIcon />
                Approve
              </FormActionButton>
            </form>

            <form
              action={issueFinitePrivateApiKeyAction}
              className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4"
            >
              <div className="flex items-center gap-2 font-semibold text-foreground">
                <KeyRoundIcon className="size-4" />
                Issue key
              </div>
              <div className="grid gap-2">
                <Label htmlFor="fpIssueGrantId">Grant id</Label>
                <Input id="fpIssueGrantId" name="grantId" required />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="fpIssueRawKey">Raw key</Label>
                <Input id="fpIssueRawKey" name="rawKey" type="password" required />
              </div>
              <div className="grid gap-2 md:grid-cols-2">
                <div className="grid gap-2">
                  <Label htmlFor="fpIssueProjectId">Project id</Label>
                  <Input id="fpIssueProjectId" name="projectId" />
                </div>
                <div className="grid gap-2">
                  <Label htmlFor="fpIssueRuntimeId">Runtime id</Label>
                  <Input id="fpIssueRuntimeId" name="agentRuntimeId" />
                </div>
              </div>
              <FormActionButton className="w-fit" pendingLabel="Issuing...">
                <KeyRoundIcon />
                Issue key
              </FormActionButton>
            </form>
          </div>

          <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
            <FinitePrivateGrantList grants={state.grants} />
            <FinitePrivateKeyList apiKeys={state.apiKeys} />
          </div>

          <div className="grid gap-2 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4">
            <div className="flex items-center gap-2 font-semibold text-foreground">
              <ActivityIcon className="size-4" />
              Audit
            </div>
            {latestEvents.length === 0 ? (
              <div className="ocean-empty-state">No Finite Private admin events yet.</div>
            ) : (
              <div className="grid gap-2">
                {latestEvents.map((event) => (
                  <div
                    key={event.id}
                    className="grid gap-1 rounded-[var(--radius-card-inner)] border border-border bg-black/10 p-3 text-sm md:grid-cols-[minmax(0,1fr)_auto]"
                  >
                    <div className="min-w-0">
                      <div className="truncate font-mono text-foreground">{event.action}</div>
                      <div className="truncate text-xs text-muted-foreground">
                        {event.target_type} / {event.target_id}
                      </div>
                    </div>
                    <div className="font-mono text-xs text-muted-foreground">
                      {shortValue(event.created_at, 19)}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      ) : null}
    </section>
  );
}

function FinitePrivateGrantList({ grants }: { grants: CoreFinitePrivateGrant[] }) {
  return (
    <div className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4">
      <div className="flex items-center gap-2 font-semibold text-foreground">
        <ShieldCheckIcon className="size-4" />
        Grants
      </div>
      {grants.length === 0 ? (
        <div className="ocean-empty-state">No grants yet.</div>
      ) : (
        <div className="grid gap-3">
          {grants.map((grant) => (
            <div
              key={grant.id}
              className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-black/10 p-3"
            >
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="truncate font-mono text-sm text-foreground">{grant.id}</span>
                  <StatusPill status={grant.status} />
                </div>
                <div className="mt-1 grid gap-1 text-xs text-muted-foreground">
                  <span className="truncate font-mono">user {grant.user_id}</span>
                  <span className="truncate font-mono">profile {grant.limit_profile_id}</span>
                  <span>{grant.current_window_used_units} used units</span>
                </div>
              </div>
              <div className="flex flex-wrap gap-2">
                <form action={resetFinitePrivateGrantAction}>
                  <input type="hidden" name="grantId" value={grant.id} />
                  <FormActionButton variant="outline" size="sm" pendingLabel="Resetting...">
                    <RotateCcwIcon />
                    Reset
                  </FormActionButton>
                </form>
                <form action={revokeFinitePrivateGrantAction}>
                  <input type="hidden" name="grantId" value={grant.id} />
                  <FormActionButton variant="outline" size="sm" pendingLabel="Revoking...">
                    <BanIcon />
                    Revoke
                  </FormActionButton>
                </form>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function FinitePrivateKeyList({ apiKeys }: { apiKeys: CoreFinitePrivateApiKey[] }) {
  return (
    <div className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4">
      <div className="flex items-center gap-2 font-semibold text-foreground">
        <KeyRoundIcon className="size-4" />
        Keys
      </div>
      {apiKeys.length === 0 ? (
        <div className="ocean-empty-state">No keys yet.</div>
      ) : (
        <div className="grid gap-3">
          {apiKeys.map((apiKey) => (
            <div
              key={apiKey.id}
              className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-black/10 p-3"
            >
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="truncate font-mono text-sm text-foreground">{apiKey.id}</span>
                  <StatusPill status={apiKey.status} />
                </div>
                <div className="mt-1 grid gap-1 text-xs text-muted-foreground">
                  <span className="truncate font-mono">grant {apiKey.grant_id}</span>
                  <span className="truncate font-mono">hash {shortValue(apiKey.key_hash, 16)}</span>
                  {apiKey.project_id ? (
                    <span className="truncate font-mono">project {apiKey.project_id}</span>
                  ) : null}
                  {apiKey.agent_runtime_id ? (
                    <span className="truncate font-mono">runtime {apiKey.agent_runtime_id}</span>
                  ) : null}
                </div>
              </div>
              <div className="grid gap-2">
                <form action={rotateFinitePrivateApiKeyAction} className="flex flex-wrap gap-2">
                  <input type="hidden" name="keyId" value={apiKey.id} />
                  <Input
                    className="min-w-48 flex-1"
                    name="rawKey"
                    type="password"
                    placeholder="Replacement raw key"
                    required
                  />
                  <FormActionButton variant="outline" size="sm" pendingLabel="Rotating...">
                    <RotateCcwIcon />
                    Rotate
                  </FormActionButton>
                </form>
                <form action={revokeFinitePrivateApiKeyAction}>
                  <input type="hidden" name="keyId" value={apiKey.id} />
                  <FormActionButton variant="outline" size="sm" pendingLabel="Revoking...">
                    <BanIcon />
                    Revoke key
                  </FormActionButton>
                </form>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function StatusPill({ status }: { status: "active" | "revoked" }) {
  return (
    <span className="rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">
      {status}
    </span>
  );
}

function CoreProjectsPanel({
  projects,
  agentCreationRequests,
}: {
  projects: CoreVisibleProject[];
  agentCreationRequests: CoreAgentCreationRequestSummary[];
}) {
  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          <ServerIcon className="size-5" />
        </span>
        <div>
          <h1 className="ocean-utility-card__title">Agent</h1>
          <p className="text-sm text-muted-foreground">
            Manage your agent and open its Hosted Web Device, with Electron and native Devices
            joining the same Finite Chat room later.
          </p>
        </div>
      </div>

      <div className="grid gap-3">
        {projects.map((project) => (
          <CoreProjectCard
            key={project.project.id}
            project={project}
            request={coreAgentCreationRequestForProject(project, agentCreationRequests)}
          />
        ))}
      </div>
    </section>
  );
}

function CoreProjectCard({
  project,
  request,
}: {
  project: CoreVisibleProject;
  request: CoreAgentCreationRequestSummary | null;
}) {
  const overviewHref = coreProjectOverviewHref(project);
  const statusLabel = coreProjectLaunchStatusLabel(project, request);
  const locationLabel = coreProjectLocationLabel(project, request);

  return (
    <div className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4 md:grid-cols-[minmax(0,1fr)_auto]">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          {overviewHref ? (
            <Link href={overviewHref} className="truncate font-semibold text-foreground hover:underline">
              {coreProjectLabel(project)}
            </Link>
          ) : (
            <h2 className="truncate font-semibold text-foreground">{coreProjectLabel(project)}</h2>
          )}
          {statusLabel ? (
            <span className="rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">
              {statusLabel}
            </span>
          ) : null}
        </div>
        <p className="mt-1 truncate font-mono text-xs text-muted-foreground">
          {locationLabel}
        </p>
        {request?.status === "failed" ? (
          <p className="mt-2 text-sm text-muted-foreground">
            We could not start this agent. Ask a team member to retry it.
          </p>
        ) : null}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {overviewHref ? (
          <Button asChild variant="outline" size="sm">
            <Link href={overviewHref}>
              <ServerIcon />
              Agent
            </Link>
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function CoreAgentCreationStatusPanel({
  requests,
}: {
  requests: CoreAgentCreationRequestSummary[];
}) {
  const first = requests[0];
  const waitingForCapacity = first ? agentCreationHasWaitedForCapacity(first) : false;
  const title = waitingForCapacity
    ? "Waiting for runner capacity"
    : first?.status === "launching"
      ? "Starting your agent"
      : "Creating your agent";
  const description = waitingForCapacity
    ? "All runner hosts are busy. Your agent is still queued and will start automatically when capacity opens."
    : `${first?.display_name ?? "Your agent"} will appear here when it is ready.`;

  return (
    <section className="ocean-utility-card">
      <div className="ocean-agent-spinup" role="status" aria-live="polite">
        <Loader2Icon className="size-5 animate-spin" aria-hidden />
        <div>
          <strong>{title}</strong>
          <span>{description}</span>
        </div>
      </div>
    </section>
  );
}

function agentCreationHasWaitedForCapacity(request: CoreAgentCreationRequestSummary) {
  if (request.status !== "requested") {
    return false;
  }
  const createdAt = Date.parse(request.created_at);
  if (!Number.isFinite(createdAt)) {
    return false;
  }
  return Date.now() - createdAt >= 5 * 60 * 1000;
}

function CoreAgentCreationFailedPanel({
  requests,
}: {
  requests: CoreAgentCreationRequestSummary[];
}) {
  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon ocean-utility-card__icon--amber" aria-hidden>
          <RotateCcwIcon className="size-5" />
        </span>
        <div>
          <h1 className="ocean-utility-card__title">Agent creation needs a retry</h1>
          <p className="text-sm text-muted-foreground">
            Reset the failed launch, then create the agent again.
          </p>
        </div>
      </div>
      <div className="grid gap-3">
        {requests.map((request) => (
          <form
            key={request.id}
            action={cancelFailedAgentCreationRequestAction}
            className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4 md:grid-cols-[minmax(0,1fr)_auto]"
          >
            <input type="hidden" name="requestId" value={request.id} />
            <div className="min-w-0">
              <h2 className="truncate font-semibold text-foreground">{request.display_name}</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                {request.failure_message ?? "The launch failed before a runtime came online."}
              </p>
            </div>
            <FormActionButton variant="outline" pendingLabel="Resetting...">
              <RotateCcwIcon />
              Start over
            </FormActionButton>
          </form>
        ))}
      </div>
    </section>
  );
}

function CoreImportCandidatesPanel({
  candidates,
}: {
  candidates: CoreProjectImportCandidate[];
}) {
  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon ocean-utility-card__icon--amber" aria-hidden>
          <CheckCircle2Icon className="size-5" />
        </span>
        <div>
          <h1 className="ocean-utility-card__title">Import existing bots</h1>
          <p className="text-sm text-muted-foreground">
            Add the bots already tied to this email.
          </p>
        </div>
      </div>

      <form action={claimCoreImportCandidatesAction} className="grid gap-4">
        <div className="grid gap-3">
          {candidates.map((candidate) => (
            <label
              key={candidate.id}
              className="grid cursor-pointer gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4 md:grid-cols-[auto_minmax(0,1fr)_auto]"
            >
              <input
                className="mt-1 size-4"
                type="checkbox"
                name="candidateId"
                value={candidate.id}
                defaultChecked
              />
              <span className="min-w-0">
                <span className="block truncate font-semibold text-foreground">
                  {candidate.host_facts.display_name}
                </span>
                <span className="mt-1 block truncate font-mono text-xs text-muted-foreground">
                  {candidate.source_host_id} / {candidate.source_machine_id}
                </span>
              </span>
              <span className="text-sm text-muted-foreground">
                {candidate.host_facts.runtime_status}
              </span>
            </label>
          ))}
        </div>
        <FormActionButton className="w-fit" pendingLabel="Importing...">
          <CheckCircle2Icon />
          Import selected
        </FormActionButton>
      </form>
    </section>
  );
}

function CoreAgentCreationPanel({
  error,
  draft,
  requiresAccess,
}: {
  error: string | null;
  draft: AgentOnboardingDraft | null;
  requiresAccess: boolean;
}) {
  const idempotencyKey = randomUUID();

  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          <PlusIcon className="size-5" />
        </span>
        <div>
          <h1 className="ocean-utility-card__title">Create an agent</h1>
          <p className="text-sm text-muted-foreground">
            Give your agent a name and make it yours.
          </p>
        </div>
      </div>

      <CoreAgentCreationForm
        error={error}
        idempotencyKey={draft?.idempotencyKey ?? idempotencyKey}
        initialName={draft?.displayName}
        initialPictureUrl={draft?.profilePictureUrl}
        runnerClass={draft?.runnerClass ?? defaultRunnerClass()}
        requiresAccess={requiresAccess}
        stripeConfigured={stripeBillingStatus().configured}
      />
    </section>
  );
}

function BillingSyncWaitPanel({ deadlineAtMs }: { deadlineAtMs: number }) {
  return (
    <section className="ocean-utility-card">
      <PendingRefresh
        enabled
        intervalMs={BILLING_SYNC_POLL_INTERVAL_MS}
        maxIntervalMs={BILLING_SYNC_MAX_POLL_INTERVAL_MS}
        deadlineAtMs={deadlineAtMs}
      />
      <div className="ocean-agent-spinup" role="status" aria-live="polite">
        <Loader2Icon className="size-5 animate-spin" aria-hidden />
        <div>
          <strong>Confirming your payment</strong>
          <span>
            Stripe accepted your checkout. We are syncing your subscription — this
            usually takes a few seconds.
          </span>
        </div>
      </div>
    </section>
  );
}

function BillingSyncTimeoutPanel() {
  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon ocean-utility-card__icon--amber" aria-hidden>
          <CreditCardIcon className="size-5" />
        </span>
        <div>
          <h1 className="ocean-utility-card__title">Payment received, still syncing</h1>
          <p className="text-sm text-muted-foreground">
            Stripe received your payment, but your subscription has not synced to
            your account yet. Check again in a moment, or contact support if this
            keeps happening.
          </p>
        </div>
      </div>
      <Button asChild variant="outline" className="w-fit">
        <Link href="/dashboard?billing=success">
          <RotateCcwIcon />
          Check again
        </Link>
      </Button>
    </section>
  );
}

function BillingCheckoutCancelledNotice() {
  return (
    <section className="ocean-utility-card">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon ocean-utility-card__icon--amber" aria-hidden>
          <CreditCardIcon className="size-5" />
        </span>
        <div>
          <h2 className="ocean-utility-card__title">Checkout cancelled</h2>
          <p className="text-sm text-muted-foreground">
            No charge was made. Start checkout again whenever you are ready.
          </p>
        </div>
      </div>
    </section>
  );
}

function emptyAccountMessage(core: CoreMeResult) {
  if (!core.configured) {
    return "This account does not have an agent yet.";
  }
  if (core.error) {
    return core.error;
  }
  if (core.me?.projects.length === 0 && core.me.claimable_candidates.length === 0) {
    return "Create your first agent to continue.";
  }
  return "Choose one of your agents to continue.";
}
