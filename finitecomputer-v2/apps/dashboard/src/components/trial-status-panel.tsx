import type { TrialAccess } from "@/lib/trial-types";
import { openBillingPortalAction } from "@/app/actions";
import { Button } from "@/components/ui/button";

export function TrialStatusPanel({ trial }: { trial?: TrialAccess | null }) {
  if (!trial || (!trial.blocked && trial.subscriptionStatus !== "trialing")) return null;
  return <section role="status" className="rounded-xl border bg-card p-5">
    <h2 className="font-medium">{trial.blocked ? "Your trial has ended. Agent access is paused" : "Your free trial is active"}</h2>
    <p className="mt-2 text-sm text-muted-foreground">{trial.blocked
      ? "Your agent’s home and history are preserved. Resolve payment to restore access to the same agent."
      : `Your trial${trial.periodEnd ? ` ends ${new Date(trial.periodEnd).toLocaleDateString("en-US", { timeZone: "UTC", month: "long", day: "numeric", year: "numeric" })} (UTC)` : " is active"}. Then $200/month, plus applicable tax. Cancel before the trial ends to avoid the charge.`}</p>
    <form action={openBillingPortalAction} className="mt-3"><Button variant="outline">Manage billing</Button></form>
  </section>;
}
