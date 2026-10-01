import { PendingRefresh } from "@/components/pending-refresh";
import type { CoreVisibleProject } from "@/lib/core-client";

export function PaymentRecoveryNotice({ recovery }: { recovery: CoreVisibleProject["runtime_recovery"] }) {
  if (!recovery) return null;
  const failed = recovery === "failed" || recovery === "restart_failed";
  return (
    <section role={failed ? "alert" : "status"} aria-live="polite" className="rounded-xl border bg-card p-4 text-sm">
      {/* Automatic failures are retried by FIN-151; a manual failure is terminal. */}
      <PendingRefresh enabled={recovery !== "restart_failed"} />
      {failed ? (
        <>{recovery === "failed" ? "Automatic restart needs help. Recovery will keep retrying." : "Restart needs help."} Open Agent and choose Restart agent to retry. If restart is unavailable, contact your Finite team for help with this agent.</>
      ) : recovery === "restarting"
        ? "Restarting your agent automatically. You can leave this page and return."
        : "Waiting for your agent to be ready. This page updates automatically."}
    </section>
  );
}
