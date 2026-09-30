import { PendingRefresh } from "@/components/pending-refresh";
import type { CoreVisibleProject } from "@/lib/core-client";

export function PaymentRecoveryNotice({ recovery }: { recovery: CoreVisibleProject["runtime_recovery"] }) {
  if (!recovery) return null;
  return (
    <section role={recovery === "failed" ? "alert" : "status"} aria-live="polite" className="rounded-xl border bg-card p-4 text-sm">
      <PendingRefresh enabled={recovery === "restarting"} />
      {recovery === "failed" ? (
        <>Automatic restart needs help. Open Agent and choose Restart agent to retry. If restart is unavailable, contact your Finite team for help with this agent.</>
      ) : "Restarting your agent automatically. You can leave this page and return; no separate resume action is needed."}
    </section>
  );
}
