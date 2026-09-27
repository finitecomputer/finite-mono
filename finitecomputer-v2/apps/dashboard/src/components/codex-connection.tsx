"use client";

import { MessageSquareIcon } from "lucide-react";

import { ConnectionCard } from "@/components/connection-card";
import { backupConfiguredFor, type InferenceView } from "@/lib/inference-status";

const PERSONAL_PLAN_COPY = "Use your personal ChatGPT plan. Work accounts may work if your organization allows it.";

/** Read-only in PR1: what this agent reports about ChatGPT, and no actions. */
export function CodexConnection({ view }: { view: InferenceView | null }) {
  const card = codexCardState(view);
  return (
    <ConnectionCard
      name="ChatGPT"
      state={card.state}
      statusLabel={card.label}
      description={<span data-testid="inference-codex-line">{card.lines.join(" ")}</span>}
      icon={<MessageSquareIcon className="size-5" />}
      testId="inference-codex"
    >
      {null}
    </ConnectionCard>
  );
}

export function codexCardState(view: InferenceView | null): {
  state: "connected" | "disconnected" | "attention" | "unavailable";
  label?: string;
  lines: string[];
} {
  if (!view) return { state: "unavailable", lines: [PERSONAL_PLAN_COPY] };
  const codex = view.codex;
  if (!codex) {
    return {
      state: "unavailable",
      label: "Update needed",
      lines: [
        "This agent needs an update to connect ChatGPT here.",
        ...(view.saved.route === "openai_codex" ? ["ChatGPT is this agent's default (set in chat)."] : []),
      ],
    };
  }
  const operation = view.operation;
  if (operation?.kind === "disconnect" && operation.route === "openai_codex" && operation.state !== "succeeded") {
    return {
      state: "attention",
      lines: [
        operation.state === "running"
          ? "Removing ChatGPT from this agent…"
          : "Removing ChatGPT didn't finish. It may still be in use.",
      ],
    };
  }
  const backup = backupConfiguredFor(view, "openai_codex") ? ["Meanwhile Finite Private may answer."] : [];
  switch (codex.state) {
    case "not_signed_in":
      return { state: "disconnected", lines: [PERSONAL_PLAN_COPY] };
    case "signed_in":
      return {
        state: "connected",
        label: "Signed in",
        lines: [
          "Signed in to ChatGPT.",
          ...(codex.reportedQuotaResetAtMs
            ? [`ChatGPT reported a usage limit until ${formatTime(codex.reportedQuotaResetAtMs)}.`]
            : []),
        ],
      };
    case "sign_in_required":
      return { state: "attention", lines: ["ChatGPT can't be used right now. Sign in again if this lasts.", ...backup] };
    case "quota_limited":
      return {
        state: "attention",
        lines: [
          codex.quotaResetAtMs
            ? `ChatGPT usage limit reached until ${formatTime(codex.quotaResetAtMs)}.`
            : "ChatGPT usage limit reached.",
          ...backup,
        ],
      };
    default:
      return { state: "unavailable", lines: ["Couldn't read the ChatGPT sign-in."] };
  }
}

function formatTime(ms: number) {
  return new Date(ms).toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
}
