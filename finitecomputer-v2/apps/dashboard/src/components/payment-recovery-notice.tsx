import { PendingRefresh } from "@/components/pending-refresh";
import type { CoreVisibleProject } from "@/lib/core-client";

export function PaymentRecoveryNotice({ recovery }: { recovery: CoreVisibleProject["runtime_recovery"] }) {
  if (!recovery) return null;
  const failed = recovery === "failed" || recovery === "restart_failed";
  return (
    <section role={failed ? "alert" : "status"} aria-live="polite" className="rounded-xl border bg-card p-4 text-sm">
      <PendingRefresh enabled={!failed} />
      {failed ? (
        <>{recovery === "failed" ? "Automatic restart needs help." : "Restart needs help."} Open Agent and choose Restart agent to retry. If restart is unavailable, contact your Finite team for help with this agent.</>
      ) : recovery === "restarting"
        ? "Restarting your agent automatically. You can leave this page and return."
        : "Your agent is restarting. This page updates automatically."}
    </section>
  );
}
