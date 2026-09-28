"use client";

import { useEffect, useId, useState, type ReactNode } from "react";
import { CpuIcon, RefreshCwIcon } from "lucide-react";

import { CodexConnection, showCodexCard } from "@/components/codex-connection";
import { ConnectionCard } from "@/components/connection-card";
import { OpenRouterConnection } from "@/components/openrouter-connection";
import { Button } from "@/components/ui/button";
import type {
  AgentConnectionAction,
  AgentConnectionActionResult,
  AgentConnectionsStatus,
} from "@/lib/hosted-agent-controls";
import {
  backupConfiguredFor,
  inferenceView,
  pollDelayMs,
  routeLabel,
  type InferenceRoute,
  type InferenceView,
  type OperationView,
} from "@/lib/inference-status";

export const STORAGE_ONLY_COPY =
  "Key saved. The agent default is unchanged. OpenRouter conversations may use it.";
/** R10b: longer than a worst-case disconnect (5 to 7 minutes), so a normal operation finishes while watched. */
export const POLL_LIMIT_MS = 8 * 60_000;

export type InferenceOutcome =
  | { ok: true; result: AgentConnectionActionResult["result"] }
  | { ok: false; code: string | null; message: string };
export type InferenceSend = (action: AgentConnectionAction) => Promise<InferenceOutcome>;
export type InferenceCard = "finite_private" | "openrouter" | "summary";
/** A synchronous result shown next to the action that produced it (§10.5 command error copy). */
export type InferenceNotice =
  | { card: InferenceCard; kind: "error"; code: string | null; message: string; route: InferenceRoute; retryModel?: string }
  | { card: InferenceCard; kind: "stored" };
/**
 * R10/R10a: while an operation runs or a disconnect has failed, every control that would start another change is
 * disabled. `reasonId` is the operation line, which each such control names with `aria-describedby`.
 */
export type InferenceLock = { locked: boolean; reasonId: string };
/** Runs an action for a card and records the notice it produces. */
export type InferenceRun = (
  card: InferenceCard,
  action: AgentConnectionAction,
  context: { route: InferenceRoute; retryModel?: string }
) => Promise<InferenceOutcome>;

const OPERATION_ERROR_COPY: Record<string, string> = {
  config_conflict: "settings changed while saving, maybe from chat",
  supervisor_unavailable: "the agent didn't restart cleanly",
  helper_unavailable: "the agent couldn't finish cleaning up",
  verify_failed: "the change didn't stick",
  config_invalid: "the change isn't valid on this agent",
};

const BACKUP_REASON_COPY: Record<string, string> = {
  settings_missing: "Finite Private isn't set up on this agent",
  credential_missing: "this agent has no Finite Private credential",
  stale_config: "it updates the next time the agent restarts",
};

export function InferenceConnections({
  status,
  busy,
  send,
  refresh,
}: {
  status: AgentConnectionsStatus | null;
  busy: boolean;
  send: InferenceSend;
  refresh: (quiet?: boolean) => Promise<void>;
}) {
  const view = status ? inferenceView(status) : null;
  const [notice, setNotice] = useState<InferenceNotice | null>(null);
  const [acceptedId, setAcceptedId] = useState<string | null>(null);
  const [seenRunningId, setSeenRunningId] = useState<string | null>(null);
  // The operation or sign-in whose polling window ran out; polling resumes for anything else.
  const [expiredPollKey, setExpiredPollKey] = useState<string | null>(null);
  const [visible, setVisible] = useState(true);

  const operationLineId = useId();
  const operation = view?.operation ?? null;
  if (operation?.state === "running" && operation.id !== seenRunningId) {
    setSeenRunningId(operation.id);
  }
  const showDone =
    operation?.state === "succeeded" && (operation.id === seenRunningId || operation.id === acceptedId);

  useEffect(() => {
    const update = () => setVisible(document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);

  const { delay, pollKey, pollExpired, locked } = watchState(view, visible, expiredPollKey);
  const lock: InferenceLock = { locked, reasonId: operationLineId };
  useEffect(() => {
    if (delay === null || pollExpired) return;
    const startedAt = Date.now();
    let inFlight = false;
    const timer = window.setInterval(async () => {
      if (Date.now() - startedAt >= POLL_LIMIT_MS) {
        setExpiredPollKey(pollKey);
        return;
      }
      if (inFlight) return;
      inFlight = true;
      try { await refresh(true); } finally { inFlight = false; }
    }, delay);
    return () => window.clearInterval(timer);
  }, [delay, pollExpired, pollKey, refresh]);

  const run: InferenceRun = async (card, action, context) => {
    setNotice(null);
    const outcome = await send(action);
    if (!outcome.ok) {
      if (
        outcome.code === "activation_not_recorded" ||
        outcome.code === "operation_in_progress" ||
        outcome.code === "agent_update_required"
      ) {
        await refresh(true);
      }
      setNotice({ card, kind: "error", code: outcome.code, message: outcome.message, ...context });
      return outcome;
    }
    const result = outcome.result;
    if (result && "accepted" in result) setAcceptedId(result.operation_id);
    if (result && "activated" in result) setNotice({ card, kind: "stored" });
    return outcome;
  };

  const retry = view && operation ? retryAction(view, operation) : null;
  return (
    <>
      <section className="ocean-inference-summary" data-testid="inference-summary-card">
        <p data-testid="inference-summary">{view ? summaryText(view) : "Checking which model new conversations use…"}</p>
        <p className="ocean-inference-summary__hint" data-testid="inference-model-hint">
          In chat, <code>/model &lt;model&gt;</code> <code>--provider</code>{" "}
          <code>&lt;{modelHintProviders(view)}&gt;</code> switches only that conversation. Add{" "}
          <code>--global</code> to change this default.
        </p>
        {view ? (
          <p className="ocean-inference-summary__line" data-testid="inference-backup-line">
            {backupLine(view)}
          </p>
        ) : null}
        <OperationLine
          id={operationLineId}
          view={view}
          showDone={showDone}
          pollExpired={pollExpired}
          busy={busy}
          onRetry={retry ? () => void run("summary", retry.action, retry.context) : null}
          onCheckAgain={() => void checkAgain(refresh, () => setExpiredPollKey(null))}
        />
        {notice?.card === "summary" && view ? <NoticeLine notice={notice} view={view} testId="inference-summary-notice" /> : null}
      </section>

      <FinitePrivateCard
        view={view}
        busy={busy}
        lock={lock}
        run={run}
        notice={notice?.card === "finite_private" ? notice : null}
      />
      <OpenRouterConnection
        view={view}
        busy={busy}
        lock={lock}
        run={run}
        notice={notice?.card === "openrouter" ? notice : null}
      />
      {view && showCodexCard(view) ? <CodexConnection view={view} /> : null}
    </>
  );
}

export function FinitePrivateCard({
  view,
  busy,
  lock,
  run,
  notice,
}: {
  view: InferenceView | null;
  busy: boolean;
  lock: InferenceLock;
  run: InferenceRun;
  notice: InferenceNotice | null;
}) {
  const card = finitePrivateCardState(view);
  return (
    <ConnectionCard
      name="Finite Private"
      state={card.state}
      statusLabel={card.label}
      account={view?.saved.route === "finite_private" && view.saved.model ? view.saved.model : null}
      description="Finite's own private model service."
      error={card.reason}
      icon={<CpuIcon className="size-5" />}
      testId="inference-finite-private"
      footer={notice && view ? <NoticeLine notice={notice} view={view} testId="inference-finite-private-notice" /> : null}
    >
      {view?.saved.route !== "finite_private" ? (
        <Button
          variant="outline"
          disabled={busy || !view || lock.locked}
          aria-describedby={lock.locked ? lock.reasonId : undefined}
          data-testid="inference-finite-private-use"
          onClick={() =>
            view && void run("finite_private", finitePrivateAction(view), { route: "finite_private" })
          }
        >
          Use Finite Private
        </Button>
      ) : null}
    </ConnectionCard>
  );
}

export function finitePrivateCardState(view: InferenceView | null): {
  state: "connected" | "disconnected" | "attention" | "unavailable";
  label?: string;
  reason: string | null;
} {
  if (!view) return { state: "unavailable", reason: null };
  if (!view.v2) {
    // Today's agentd reports no Finite Private facts; say only what the saved route shows.
    return view.saved.route === "finite_private"
      ? { state: "connected", label: "Agent default", reason: null }
      : { state: "disconnected", label: "Not the agent default", reason: null };
  }
  if (view.finitePrivate.state === "configured") return { state: "connected", label: "Configured", reason: null };
  const reason = view.finitePrivate.reason ? BACKUP_REASON_COPY[view.finitePrivate.reason] : undefined;
  return { state: "attention", reason: reason ? `${reason[0].toUpperCase()}${reason.slice(1)}.` : null };
}

export function finitePrivateAction(view: InferenceView): AgentConnectionAction {
  return view.capabilities.has("inference.select.v1")
    ? { action: "inference_select", route: "finite_private" }
    : { action: "inference", profile: "finite_private" };
}

/** R9a: the `/model` hint names `openai-codex` only when the ChatGPT card is shown. */
export function modelHintProviders(view: InferenceView | null) {
  return view && showCodexCard(view) ? "finite-private|openrouter|openai-codex" : "finite-private|openrouter";
}

/**
 * R10, R10a, R10b: the lock follows the last status the page has. It is locked while that status shows an
 * operation running, or a failed disconnect, because agentd refuses every other change until then. The polling
 * limit unlocks nothing; "Check again" and "Try again" are never locked, so the panel can't be stuck. A failed
 * select or activate locks nothing: the next change replaces it.
 */
export function controlsLocked(view: InferenceView | null) {
  const operation = view?.operation;
  if (operation?.state === "running") return true;
  return operation?.kind === "disconnect" && operation.state === "failed";
}

/**
 * What the page polls for, whether that polling window has run out, and whether controls are locked.
 * `expiredPollKey` is the operation (or sign-in attempt) whose window ran out; polling runs again for anything
 * else, or once "Check again" clears it. The lock comes from the last status alone (R10b): an expired window
 * never unlocks it.
 */
export function watchState(view: InferenceView | null, visible: boolean, expiredPollKey: string | null) {
  const operation = view?.operation ?? null;
  const pollKey = operation?.state === "running" ? operation.id : (view?.codex?.login?.attemptId ?? null);
  const pollExpired = pollKey !== null && pollKey === expiredPollKey;
  const delay = view && !pollExpired ? pollDelayMs(view, visible) : null;
  return { delay, pollKey, pollExpired, locked: controlsLocked(view) };
}

/**
 * R10b: "Check again" reads status once, then clears the expired polling window. If that status still shows the
 * operation running, the same poll key is live again and polling resumes; if it has ended, there is nothing to poll.
 */
export async function checkAgain(refresh: () => Promise<void>, resumePolling: () => void) {
  try {
    await refresh();
  } finally {
    resumePolling();
  }
}

/** R11: the one line a route's card shows while that route is being removed, or null. */
export function removalText(view: InferenceView, route: "openrouter" | "openai_codex") {
  const operation = view.operation;
  if (operation?.kind !== "disconnect" || operation.route !== route || operation.state === "succeeded") return null;
  const label = routeLabel(route);
  return operation.state === "running"
    ? `Removing ${label} from this agent…`
    : `Removing ${label} didn't finish. It may still be in use.`;
}

export function summaryText(view: InferenceView): ReactNode {
  const { route, model } = view.saved;
  const label = routeLabel(route);
  const end = route === "other" ? "," : ".";
  // A model ID wraps as one piece, with its punctuation, unless it alone is wider than the card.
  const choice = model ? (
    <>
      <strong>{label} · </strong>
      <span className="ocean-inference-id">
        <strong>{model}</strong>
        {end}
      </span>
    </>
  ) : (
    <>
      <strong>{label}</strong>
      {end}
    </>
  );
  return route === "other" ? (
    <>New conversations use {choice} set in Hermes. Finite can&apos;t manage this provider here.</>
  ) : (
    <>New conversations use {choice}</>
  );
}

export function backupLine(view: InferenceView) {
  const { fallback } = view;
  if (backupConfiguredFor(view, view.saved.route)) {
    const base =
      view.saved.route === "finite_private"
        ? "Finite Private backup is configured for conversations that use another model."
        : `Finite Private backup is configured. If ${routeLabel(view.saved.route)} returns an error, Finite Private answers.`;
    return fallback.extraEntries > 0 ? `${base} Your other backup models are tried after it.` : base;
  }
  const reason = fallback.reason ? BACKUP_REASON_COPY[fallback.reason] : undefined;
  switch (fallback.state) {
    case "unavailable":
      return reason ? `Finite Private can't step in right now: ${reason}.` : "Finite Private can't step in right now.";
    case "not_configured":
      return `Finite Private backup isn't set up on this agent${reason ? `: ${reason}` : ""}.`;
    case "off":
      return "Backup to Finite Private is turned off in Hermes.";
    case "custom":
      return "Backup is customized in Hermes.";
    default:
      return "Backup details aren't available on this agent yet.";
  }
}

export function operationText(operation: OperationView, showDone: boolean) {
  const label = routeLabel(operation.route);
  const paused =
    operation.kind === "disconnect" && operation.state !== "succeeded"
      ? " The agent's Hermes web dashboard is paused until this finishes."
      : "";
  if (operation.state === "succeeded") return showDone ? "Done." : null;
  if (operation.kind === "disconnect") {
    return operation.state === "running"
      ? `Disconnecting ${label}… This connection may still be in use until this finishes.${paused}`
      : `Disconnecting ${label} didn't finish. It may still be in use.${paused}`;
  }
  const target = `${label}${operation.model ? ` · ${operation.model}` : ""}`;
  if (operation.state === "running") return `Switching to ${target}…`;
  const error = operation.errorCode ? OPERATION_ERROR_COPY[operation.errorCode] : undefined;
  return `Switching to ${target} didn't finish${error ? ` (${error})` : ""}. Try again.`;
}

export function OperationLine({
  id,
  view,
  showDone,
  pollExpired,
  busy,
  onRetry,
  onCheckAgain,
}: {
  id?: string;
  view: InferenceView | null;
  showDone: boolean;
  pollExpired: boolean;
  busy: boolean;
  onRetry: (() => void) | null;
  onCheckAgain: () => void;
}) {
  const operation = view?.operation ?? null;
  const text = operation ? operationText(operation, showDone) : null;
  return (
    <div id={id} className="ocean-inference-summary__operation" role="status" aria-live="polite">
      {text ? <span data-testid="inference-operation-line">{text}</span> : null}
      {operation?.state === "failed" && onRetry ? (
        <Button size="sm" variant="outline" disabled={busy} data-testid="inference-operation-retry" onClick={onRetry}>
          Try again
        </Button>
      ) : null}
      {operation?.state === "running" && pollExpired ? (
        <Button size="sm" variant="outline" data-testid="inference-operation-check-again" onClick={onCheckAgain}>
          <RefreshCwIcon />
          Check again
        </Button>
      ) : null}
    </div>
  );
}

/** Retrying a failed operation repeats it; a failed disconnect is resumed by agentd (§3.11). */
function retryAction(
  view: InferenceView,
  operation: OperationView
): { action: AgentConnectionAction; context: { route: InferenceRoute } } | null {
  if (operation.state !== "failed") return null;
  if (operation.kind === "disconnect") {
    if (!view.capabilities.has("inference.disconnect.v1")) return null;
    if (operation.route !== "openrouter" && operation.route !== "openai_codex") return null;
    return { action: { action: "inference_disconnect", route: operation.route }, context: { route: operation.route } };
  }
  if (!view.capabilities.has("inference.select.v1")) return null;
  if (operation.route === "finite_private") {
    return { action: { action: "inference_select", route: "finite_private" }, context: { route: "finite_private" } };
  }
  if ((operation.route === "openrouter" || operation.route === "openai_codex") && operation.model) {
    return {
      action: { action: "inference_select", route: operation.route, model: operation.model },
      context: { route: operation.route },
    };
  }
  return null;
}

export function commandErrorText(
  code: string | null,
  message: string,
  route: InferenceRoute,
  view: InferenceView
) {
  const label = routeLabel(route);
  switch (code) {
    case "not_connected":
      return `Connect ${label} first.`;
    case "sign_in_required":
      return "Sign in to ChatGPT again first.";
    case "credential_rejected":
      return message || "OpenRouter didn't accept this key.";
    case "key_allowance_exhausted":
      return "This key has no remaining allowance. Raise its limit at openrouter.ai/keys, or use another key.";
    case "provider_unavailable":
      return `Couldn't reach ${label}. Nothing changed. Try again.`;
    case "catalog_unavailable":
      return "ChatGPT's model list is unavailable right now. Your saved model is kept.";
    case "model_unavailable":
      return "That model isn't available for this ChatGPT account.";
    case "attempt_not_found":
      return "That sign-in is no longer active. Start again.";
    case "operation_in_progress": {
      const running = view.operation?.state === "running" ? operationText(view.operation, false) : null;
      return running ? `Another connection change is still finishing. ${running}` : "Another connection change is still finishing.";
    }
    case "disconnect_in_progress":
      return "ChatGPT is being removed from this agent. Wait for that to finish, or try the removal again.";
    case "finite_private_unavailable":
      return "Disconnecting would leave this agent without a model: Finite Private isn't fully set up here. Choose another model first.";
    case "facts_unavailable":
      return "The agent couldn't check its setup right now. Try again in a moment.";
    case "activation_not_recorded":
      return "The key was saved, but the agent couldn't record the switch to OpenRouter.";
    case "agent_update_required":
      return "This agent needs an update for this.";
    default:
      return message;
  }
}

export function NoticeLine({
  notice,
  view,
  testId,
  onRetryUse,
  busy = false,
  describedBy,
}: {
  notice: InferenceNotice;
  view: InferenceView;
  testId: string;
  onRetryUse?: (model: string) => void;
  busy?: boolean;
  describedBy?: string;
}) {
  if (notice.kind === "stored") {
    return (
      <p className="text-sm" role="status" aria-live="polite" data-testid={testId}>
        {STORAGE_ONLY_COPY}
      </p>
    );
  }
  const retryModel = notice.code === "activation_not_recorded" ? notice.retryModel : undefined;
  return (
    <div className="flex flex-wrap items-center gap-2" role="alert" data-testid={testId}>
      <p className="text-sm text-destructive">{commandErrorText(notice.code, notice.message, notice.route, view)}</p>
      {retryModel && onRetryUse ? (
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          aria-describedby={describedBy}
          data-testid="inference-openrouter-retry-use"
          onClick={() => onRetryUse(retryModel)}
        >
          Use OpenRouter
        </Button>
      ) : null}
    </div>
  );
}
