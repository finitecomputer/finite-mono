import type { AgentConnectionsStatus } from "@/lib/hosted-agent-controls";

export type InferenceRoute = "finite_private" | "openrouter" | "openai_codex" | "other";
export type Tri = "present" | "absent" | "unknown";
export type CodexLoginView = {
  attemptId: string;
  state: "pending" | "committing" | "approved" | "canceled" | "expired" | "failed" | "interrupted";
  userCode: string | null; verificationUri: string | null; expiresAtMs: number; pollIntervalS: number;
  errorCode: string | null; retryAfterS: number | null };
export type OperationView = {
  id: string; kind: "select" | "activate" | "disconnect"; route: InferenceRoute; model: string | null;
  state: "running" | "succeeded" | "failed"; phase: string; errorCode: string | null; attempts: number; updatedAtMs: number };
export type InferenceView = {
  v2: boolean;                                   // capabilities includes "inference.status.v2"
  saved: { route: InferenceRoute; provider: string | null; model: string | null };
  finitePrivate: { state: "configured" | "not_configured" | "unknown"; reason: string | null };
  openrouter: { state: "key_saved" | "no_key" | "unknown"; keySource: "agent" | "legacy_config" | "environment" | null;
                keyHash: string | null; hermesKey: "saved_key" | "other_key" | "none" | "unknown"; otherPoolKeys: "none" | "present" | "unknown" };
  codex: { state: "not_signed_in" | "signed_in" | "quota_limited" | "sign_in_required" | "unknown";
           quotaResetAtMs: number | null; reportedQuotaResetAtMs: number | null; login: CodexLoginView | null } | null;   // null = unsupported
  fallback: { state: "configured" | "unavailable" | "not_configured" | "off" | "custom" | "unknown";
              reason: string | null; model: string | null; extraEntries: number };
  operation: OperationView | null;
  capabilities: ReadonlySet<string>;
};

const POLL_DELAY_MS = 3_000;

/** Old agentd reports Codex as `profile: finite_private`; only the raw provider is trusted without v2. */
export function classifyLegacyProvider(provider: string): InferenceRoute {
  switch (provider) {
    case "openrouter":
      return "openrouter";
    case "openai-codex":
    case "codex":
    case "openai_codex":
      return "openai_codex";
    case "custom":
    case "finite-private":
    case "custom:finite-private":
      return "finite_private";
    default:
      return "other";
  }
}

export function inferenceView(status: AgentConnectionsStatus): InferenceView {
  const capabilities = new Set(status.capabilities ?? []);
  const v2 = capabilities.has("inference.status.v2");
  const inference = status.inference;
  const routes = v2 ? inference.routes : undefined;
  const saved = v2 ? inference.saved : undefined;
  const fallback = v2 ? inference.fallback : undefined;
  const openrouter = routes?.openrouter;
  const codex = routes?.openai_codex;
  const operation = inference.operation;
  return {
    v2,
    saved: saved
      ? { route: saved.route, provider: saved.provider, model: saved.model }
      : {
          route: classifyLegacyProvider(inference.provider),
          provider: inference.provider,
          model: inference.model,
        },
    finitePrivate: {
      state: routes?.finite_private?.state ?? "unknown",
      reason: routes?.finite_private?.reason ?? null,
    },
    openrouter: {
      state: openrouter?.state ?? "unknown",
      keySource: openrouter?.key_source ?? null,
      keyHash: openrouter?.key_hash ?? null,
      hermesKey: openrouter?.hermes_key ?? "unknown",
      otherPoolKeys: openrouter?.other_pool_keys ?? "unknown",
    },
    codex: capabilities.has("codex.login.v1")
      ? {
          state: codex?.state ?? "unknown",
          quotaResetAtMs: codex?.quota_reset_at_ms ?? null,
          reportedQuotaResetAtMs: codex?.reported_quota_reset_at_ms ?? null,
          login: codex?.login
            ? {
                attemptId: codex.login.attempt_id,
                state: codex.login.state,
                userCode: codex.login.user_code,
                verificationUri: codex.login.verification_uri,
                expiresAtMs: codex.login.expires_at_ms,
                pollIntervalS: codex.login.poll_interval_s,
                errorCode: codex.login.error_code,
                retryAfterS: codex.login.retry_after_s,
              }
            : null,
        }
      : null,
    fallback: {
      state: fallback?.state ?? "unknown",
      reason: fallback?.reason ?? null,
      model: fallback?.model ?? null,
      extraEntries: fallback?.extra_entries ?? 0,
    },
    operation: operation
      ? {
          id: operation.id,
          kind: operation.kind,
          route: operation.route,
          model: operation.model,
          state: operation.state,
          phase: operation.phase,
          errorCode: operation.error_code,
          attempts: operation.attempts,
          updatedAtMs: operation.updated_at_ms,
        }
      : null,
    capabilities,
  };
}

export function routeLabel(route: InferenceRoute): string {
  switch (route) {
    case "finite_private":
      return "Finite Private";
    case "openrouter":
      return "OpenRouter";
    case "openai_codex":
      return "ChatGPT";
    case "other":
      return "Custom model";
  }
}

export function backupConfiguredFor(view: InferenceView, route: InferenceRoute): boolean {
  return view.fallback.state === "configured" && view.saved.route === route;
}

export function pollDelayMs(view: InferenceView, visible: boolean): number | null {
  if (!visible) return null;
  const loginState = view.codex?.login?.state;
  return view.operation?.state === "running" || loginState === "pending" || loginState === "committing"
    ? POLL_DELAY_MS
    : null;
}
