import { TicketIcon } from "lucide-react";
import { loadCoreTrialCampaigns } from "@/lib/core-client";
import { summarizeTrialCampaigns, trialCampaignStatus, trialDate, trialSeatStatus } from "@/lib/admin-trials";
import type { TrialCampaign } from "@/lib/trial-types";
import { AdminTrialCapacityForm, AdminTrialForm, AdminTrialRefreshButton } from "@/components/admin-trial-form";

export async function AdminTrialsPanel() {
  const enabled = process.env.FC_DASHBOARD_TRIALS_ENABLED === "true";
  const result = await loadCoreTrialCampaigns().then(
    campaigns => ({ campaigns, error: null }),
    () => ({ campaigns: null, error: "Trial campaigns are unavailable. Refresh to retry or check Core availability and admin access." }),
  );
  return <section className="ocean-utility-card grid min-w-0 gap-6">
    <div className="ocean-utility-card__header">
      <span className="ocean-utility-card__icon" aria-hidden><TicketIcon className="size-5" /></span>
      <div><h2 className="ocean-utility-card__title">Free trials</h2>
        <p className="text-sm text-muted-foreground">Campaign codes, signup capacity and trial access. Counts reflect Core at page load; refresh for the latest.</p></div>
    </div>
    <AdminTrialRefreshButton />
    {result.error ? <p role="alert" className="ocean-empty-state">{result.error}</p> : null}
    {result.campaigns ? <>
      <TrialSummary campaigns={result.campaigns} />
      {enabled ? <AdminTrialForm /> : <p className="ocean-empty-state">Trial checkout is disabled in this dashboard. Campaign creation and capacity changes are unavailable here.</p>}
      <div className="grid gap-4" aria-label="Trial campaigns">
        {result.campaigns.length === 0 ? <p className="ocean-empty-state">No free trial campaigns yet.</p> : null}
        {result.campaigns.map(campaign => <CampaignCard key={campaign.id} campaign={campaign} enabled={enabled} />)}
      </div>
    </> : null}
    <section className="grid gap-3 border-t border-border pt-5" aria-labelledby="trial-readiness">
      <h3 id="trial-readiness" className="font-medium">Readiness checks</h3>
      <dl className="grid gap-3 text-sm sm:grid-cols-2">
        <Readiness label="Dashboard trial checkout" state={enabled ? "Verified enabled" : "Verified disabled"} detail="Current dashboard configuration only." />
        <Readiness label="Core campaign read" state={result.campaigns ? "Verified" : "Unknown"} detail={result.campaigns ? "Authorized campaign response received for this page load." : "No successful campaign response."} />
        <Readiness label="Live billing and expiry delivery" state="Unknown" detail="Stripe configuration, webhooks and end-to-end expiry are not probed by this page." />
        <Readiness label="Physical runtime capacity" state="Unknown" detail="Campaign seats are signup capacity. They do not measure free runtime slots or archival." />
      </dl>
      <details className="text-sm text-muted-foreground">
        <summary className="cursor-pointer">How trial states and limits work</summary>
        <div className="mt-3 grid gap-2">
          <p>Account registration is separate from trial checkout. Campaigns attribute checkout accounts, not every account signup. Reserved checkouts and completed redemptions both occupy seats.</p>
          <p>Checkout expiry releases only a reserved seat after verified Stripe expiry. A redeemed seat stays used after the trial ends, payment fails or a subscription is canceled.</p>
          <p>For standard trial accounts, Core allows access while trialing before the deadline, or while the subscription is active. Other billing states or elapsed trials block access. Exempt account policies remain separate.</p>
          <p>Stripe owns billing timing. The billing period end below is reported by Core; when absent, Core can use redemption time plus trial days as its access deadline. Access allowed does not prove an agent is running or ready.</p>
        </div>
      </details>
    </section>
  </section>;
}

function TrialSummary({ campaigns }: { campaigns: TrialCampaign[] }) {
  const summary = summarizeTrialCampaigns(campaigns);
  return <div className="grid gap-3">
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <Metric label="Seats used / total limit" value={`${summary.redeemed + summary.reserved} / ${summary.limit}`} />
      <Metric label="Remaining · active codes" value={summary.remaining} />
      <Metric label="Completed redemptions" value={summary.redeemed} />
      <Metric label="Active trials" value={summary.activeTrials ?? "Unknown"} />
    </div>
    <p className="text-xs text-muted-foreground">{summary.reserved} in checkout · {summary.attributedAccounts} unique attributed accounts · {summary.expiredCheckouts} expired checkout attempts</p>
    <p className="text-xs text-muted-foreground">Account signups: unknown (registration attribution is not recorded). Active trials count trialing accounts with Core access allowed.{summary.unknownAccess ? ` Access data is missing for ${summary.unknownAccess} redeemed seats.` : ""}</p>
  </div>;
}

function Metric({ label, value }: { label: string; value: string | number }) {
  return <div className="ocean-metric"><span>{value}</span><small>{label}</small></div>;
}

function CampaignCard({ campaign, enabled }: { campaign: TrialCampaign; enabled: boolean }) {
  const used = campaign.redeemedSeats + campaign.reservedSeats;
  return <article className="grid min-w-0 gap-4 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4" aria-label={campaign.name}>
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div className="min-w-0"><h3 className="break-words font-semibold">{campaign.name}</h3>
        <p className="mt-1 text-xs text-muted-foreground">{campaign.trialDays}-day trial · Code shown only at creation · No redemption deadline</p></div>
      <span className="rounded-full border border-border px-2 py-0.5 text-xs">{trialCampaignStatus(campaign)}</span>
    </div>
    <div className="grid gap-2">
      <div className="flex flex-wrap justify-between gap-2 text-sm"><span><strong>{used} / {campaign.seatLimit}</strong> seats used</span><span>{campaign.seatsRemaining} remaining{!campaign.active ? " · inactive code" : ""}</span></div>
      <div className="h-2 overflow-hidden rounded-full bg-muted ring-1 ring-inset ring-border" role="progressbar" aria-valuemin={0} aria-valuenow={used} aria-valuemax={campaign.seatLimit} aria-label={`${campaign.name} seats used`}>
        <div className="h-full rounded-full bg-emerald-500" style={{ width: `${Math.min(100, Math.max(0, used / campaign.seatLimit * 100))}%` }} />
      </div>
      <p className="text-xs text-muted-foreground">{campaign.redeemedSeats} redeemed · {campaign.reservedSeats} in checkout · {summarizeTrialCampaigns([campaign]).attributedAccounts} attributed accounts</p>
    </div>
    {enabled && campaign.active && campaign.seatLimit < 10000 ? <AdminTrialCapacityForm campaignId={campaign.id} seatLimit={campaign.seatLimit} /> : null}
    <details className="min-w-0 text-sm">
      <summary className="cursor-pointer">Account attribution and trial states ({campaign.redemptions.length})</summary>
      {campaign.redemptions.length === 0 ? <p className="mt-3 text-muted-foreground">No checkout attempts yet.</p> : <div className="mt-3 overflow-x-auto">
        <table className="w-full text-left text-xs">
          <caption className="sr-only">{campaign.name} checkout attribution and billing access</caption>
          <thead className="text-muted-foreground"><tr><th className="p-2">Account</th><th className="p-2">State / access</th><th className="p-2">Redeemed at</th><th className="p-2">Billing period end</th></tr></thead>
          <tbody>{campaign.redemptions.map((seat, index) => <tr className="border-t border-border" key={`${seat.customerOrgId}-${index}`}>
            <td className="min-w-40 max-w-64 break-all p-2">{seat.ownerWorkosUserId ?? seat.customerOrgId}<span className="block text-muted-foreground">{seat.ownerWorkosUserId ? seat.customerOrgId : "Owner unknown"}</span></td>
            <td className="min-w-40 p-2">{trialSeatStatus(seat)}</td>
            <td className="min-w-32 p-2">{seat.redeemedAt ? trialDate(seat.redeemedAt) : "—"}</td>
            <td className="min-w-32 p-2">{seat.state === "redeemed" ? trialDate(seat.trialAccess?.periodEnd ?? null) : "—"}</td>
          </tr>)}</tbody>
        </table>
      </div>}
    </details>
  </article>;
}

function Readiness({ label, state, detail }: { label: string; state: string; detail: string }) {
  return <div className="rounded-lg border border-border p-3"><dt className="flex flex-wrap justify-between gap-2 font-medium">{label}<span className="text-xs text-muted-foreground">{state}</span></dt><dd className="mt-1 text-xs text-muted-foreground">{detail}</dd></div>;
}
