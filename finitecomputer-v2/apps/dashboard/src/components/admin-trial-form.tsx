"use client";

import { useActionState } from "react";
import { issueTrialCampaignAction } from "@/app/dashboard/admin/trial-actions";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function AdminTrialForm() {
  const [state, action, pending] = useActionState(issueTrialCampaignAction, {});
  return <form action={action} className="grid gap-4">
    <div className="grid gap-2"><Label htmlFor="trial-event">Event name</Label>
      <Input id="trial-event" name="name" required maxLength={120} placeholder="October summit" /></div>
    <div className="grid gap-4 sm:grid-cols-2">
      <div className="grid gap-2"><Label htmlFor="trial-seats">Seat limit</Label>
        <Input id="trial-seats" name="seatLimit" type="number" min={1} max={10000} required /></div>
      <div className="grid gap-2"><Label htmlFor="trial-days">Trial days</Label>
        <Input id="trial-days" name="trialDays" type="number" min={1} max={30} defaultValue={7} required /></div>
    </div>
    <Button disabled={pending}>{pending ? "Issuing…" : "Issue trial code"}</Button>
    {state.error ? <p role="alert">{state.error}</p> : null}
    {state.code ? <div role="status" className="rounded-lg border p-4">
      <p className="mb-2 text-sm">Save this code now. It is shown only once.</p>
      <code className="break-all select-all">{state.code}</code>
    </div> : null}
  </form>;
}
