import { MessageSquareIcon } from "lucide-react";

import { ConnectionCard } from "@/components/connection-card";
import { removalText } from "@/components/inference-connections";
import { backupConfiguredFor, type InferenceView } from "@/lib/inference-status";

const PERSONAL_PLAN_COPY = "Use your personal ChatGPT plan. Work accounts may work if your organization allows it.";

export function showCodexCard(view: InferenceView) {
  return view.saved.route === "openai_codex" || view.codex !== null;
}

export function CodexConnection({ view }: { view: InferenceView }) {
  const card = codexCardState(view);
  return (
    <ConnectionCard
      name="ChatGPT"
      state={card.state}
      statusLabel={card.label}
      description={<span data-testid="inference-codex-line">{card.lines.join(" ")}</span>}
      icon={<MessageSquareIcon className="size-5" />}
      testId="inference-codex"
    />
  );
}

function codexCardState(view: InferenceView): {
  state: "connected" | "disconnected" | "attention" | "unavailable";
  label?: string;
  lines: string[];
} {
  const codex = view.codex;
  if (!codex) {
    // Only reached when ChatGPT is the saved route (showCodexCard).
    return { state: "connected", label: "Agent default", lines: ["ChatGPT is this agent's default (set in chat)."] };
  }
  const removing = removalText(view, "openai_codex");
  if (removing) return { state: "attention", lines: [removing] };
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
