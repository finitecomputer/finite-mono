"use client";

import { useActionState, useState, useTransition } from "react";
import { useRouter } from "next/navigation";
import { increaseTrialCapacityAction, issueTrialCampaignAction } from "@/app/dashboard/admin/trial-actions";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function AdminTrialForm() {
  const [state, action, pending] = useActionState(issueTrialCampaignAction, {});
  return <details className="rounded-[var(--radius-card-inner)] border border-border p-4">
    <summary className="cursor-pointer font-medium">Create new free trial campaign</summary>
    <form action={action} className="mt-4 grid gap-4" aria-label="Create trial campaign">
      <p className="text-sm text-muted-foreground">Generate one readable code with a bounded signup limit. Codes accept lowercase and optional spaces or hyphens. Save the code after creation; it is shown only once.</p>
      <div className="grid gap-2"><Label htmlFor="trial-event">Campaign name</Label>
        <Input id="trial-event" name="name" required maxLength={120} placeholder="October workshop" /></div>
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="grid gap-2"><Label htmlFor="trial-seats">Total signup limit</Label>
          <Input id="trial-seats" name="seatLimit" type="number" min={1} max={10000} step={1} required defaultValue={10} /></div>
        <div className="grid gap-2"><Label htmlFor="trial-days">Trial days</Label>
          <Input id="trial-days" name="trialDays" type="number" min={1} max={30} step={1} defaultValue={7} required /></div>
      </div>
      <p className="text-xs text-muted-foreground">Redemption deadline: not supported by the campaign model. Existing checkout requires a payment method and uses the $200/month plan after the trial.</p>
      <Button className="w-fit max-w-full" disabled={pending}>{pending ? "Creating…" : "Create campaign and code"}</Button>
      {state.error ? <p role="alert">{state.error}</p> : null}
      {state.code ? <div role="status" className="rounded-lg border p-4">
        <p className="mb-2 font-medium">{state.message}</p>
        <p className="mb-2 text-sm">Campaign created. Save this code now. It is shown only once.</p>
        <code className="break-all select-all">{state.code}</code>
      </div> : null}
    </form>
  </details>;
}

export function AdminTrialCapacityForm({ campaignId, seatLimit }: { campaignId: string; seatLimit: number }) {
  const [state, action, pending] = useActionState(increaseTrialCapacityAction, {});
  const [input, setInput] = useState({ base: seatLimit, value: String(seatLimit + 5) });
  const total = input.base === seatLimit ? input.value : String(seatLimit + 5);
  const additional = Number(total) - seatLimit;
  const valid = Number.isInteger(Number(total)) && additional > 0 && Number(total) <= 10000;
  return <details className="text-sm">
    <summary className="cursor-pointer font-medium">Increase signup limit</summary>
    <form action={action} className="mt-3 grid max-w-md gap-3" aria-label="Increase campaign capacity">
      <input type="hidden" name="campaignId" value={campaignId} />
      <input type="hidden" name="expectedSeatLimit" value={seatLimit} />
      <Label htmlFor={`capacity-${campaignId}`}>New total signup limit</Label>
      <Input id={`capacity-${campaignId}`} name="seatLimit" type="number" min={seatLimit + 1} max={10000} step={1} required value={total} onChange={event => setInput({ base: seatLimit, value: event.target.value })} aria-describedby={`capacity-preview-${campaignId}`} />
      <p id={`capacity-preview-${campaignId}`} className="text-xs text-muted-foreground" aria-live="polite">
        {valid ? `${seatLimit} → ${total} total seats · adds ${additional} seats to this code.` : `Enter a total above ${seatLimit}, up to 10000.`}
      </p>
      <Button variant="outline" className="w-fit" disabled={pending || !valid}>{pending ? "Increasing…" : "Increase total limit"}</Button>
      {state.error ? <p role="alert">{state.error}</p> : null}
      {state.message ? <p role="status">{state.message}</p> : null}
    </form>
  </details>;
}

export function AdminTrialRefreshButton() {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  return <Button variant="outline" size="sm" className="w-fit" disabled={pending} onClick={() => startTransition(() => router.refresh())}>
    {pending ? "Refreshing…" : "Refresh counts"}
  </Button>;
}
