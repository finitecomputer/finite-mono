"use client";

import { useActionState, useState } from "react";
import { updateTrialCodeAction } from "@/app/dashboard/admin/trial-actions";
import type { TrialCampaign } from "@/lib/trial-types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function AdminTrialCode({ campaign, enabled }: { campaign: Pick<TrialCampaign, "id" | "code" | "codeRevision">; enabled: boolean }) {
  const [state, action, pending] = useActionState(updateTrialCodeAction, {});
  const [copyStatus, setCopyStatus] = useState("");
  const [draft, setDraft] = useState({ revision: campaign.codeRevision, value: campaign.code ?? "" });
  const value = draft.revision === campaign.codeRevision ? draft.value : campaign.code ?? "";
  return <div className="grid gap-3 text-sm">
    {campaign.code ? <div className="flex flex-wrap items-center gap-3">
      <code className="min-w-0 break-all select-all">{campaign.code}</code>
      <Button type="button" variant="outline" size="sm" onClick={async () => {
        try { await navigator.clipboard.writeText(campaign.code!); setCopyStatus("Code copied."); }
        catch { setCopyStatus("Could not copy. Select the code to copy it manually."); }
      }}>Copy code</Button>
      {copyStatus ? <span role="status">{copyStatus}</span> : null}
    </div> : <p className="text-muted-foreground">This code is unavailable for display. Its validity is unchanged. Set a replacement if you no longer have it.</p>}
    {enabled && campaign.codeRevision !== undefined ? <details>
      <summary className="cursor-pointer font-medium">{campaign.code ? "Edit code" : "Set replacement code"}</summary>
      <form action={action} className="mt-3 grid max-w-md gap-3" aria-label="Edit campaign code">
        <input type="hidden" name="campaignId" value={campaign.id} />
        <input type="hidden" name="expectedCodeRevision" value={campaign.codeRevision} />
        <Label htmlFor={`code-${campaign.id}`}>Trial code</Label>
        <Input id={`code-${campaign.id}`} name="code" required maxLength={128} autoComplete="off" spellCheck={false}
          value={value} onChange={event => setDraft({ revision: campaign.codeRevision, value: event.target.value })}
          aria-describedby={`code-help-${campaign.id}`} placeholder="WORKSHOP2026" />
        <p id={`code-help-${campaign.id}`} className="text-xs text-muted-foreground">Use 8–64 letters or numbers. Case, spaces and hyphens do not matter. Saving a different code stops new checkouts with the old code. Existing checkouts, enrollments and capacity are preserved.</p>
        <Button variant="outline" className="w-fit" disabled={pending}>{pending ? "Saving…" : "Save code"}</Button>
        {state.error ? <p role="alert">{state.error}</p> : null}
        {state.message ? <p role="status">{state.message}</p> : null}
      </form>
    </details> : null}
  </div>;
}
