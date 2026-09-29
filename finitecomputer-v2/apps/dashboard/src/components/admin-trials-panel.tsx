import { loadCoreTrialCampaigns } from "@/lib/core-client";
import { AdminTrialForm } from "@/components/admin-trial-form";

export async function AdminTrialsPanel() {
  const result = await loadCoreTrialCampaigns().then(campaigns => ({ campaigns, error: null }), () => ({ campaigns: [], error: "Trial campaigns are unavailable. Check Core availability." }));
  return <section className="ocean-utility-card grid gap-6 p-6">
    <div><h2 className="text-lg font-medium">Event trials</h2>
      <p className="text-sm text-muted-foreground">Issue a code for each event and track its trial seats.</p></div>
    {result.error ? <p role="alert">{result.error}</p> : <>
      <AdminTrialForm />
      {result.campaigns.map(campaign => <div key={campaign.id} className="grid gap-2 border-t pt-4">
        <h3 className="font-medium">{campaign.name}</h3>
        <p className="text-sm">{campaign.trialDays} days · {campaign.redeemedSeats} redeemed · {campaign.reservedSeats} in checkout · {campaign.seatsRemaining} of {campaign.seatLimit} seats available</p>
        <details><summary className="cursor-pointer text-sm">Seat attribution</summary>
          <ul className="mt-2 grid gap-1 text-xs">
            {campaign.redemptions.map((seat, index) => <li key={`${seat.customerOrgId}-${index}`}>
              {seat.ownerWorkosUserId ?? seat.customerOrgId} · {seat.state}{seat.redeemedAt ? ` · ${seat.redeemedAt}` : ""}
            </li>)}
          </ul>
        </details>
      </div>)}
    </>}
  </section>;
}
