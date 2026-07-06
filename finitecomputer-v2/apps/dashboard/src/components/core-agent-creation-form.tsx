"use client";

import { useRef, useState } from "react";
import { Loader2Icon, PlusIcon } from "lucide-react";

import { FormActionButton } from "@/components/form-action-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function CoreAgentCreationForm({
  error,
  idempotencyKey,
}: {
  error: string | null;
  idempotencyKey: string;
}) {
  const [submitting, setSubmitting] = useState(false);
  const submittedRef = useRef(false);

  if (submitting) {
    return (
      <div className="ocean-agent-spinup" role="status" aria-live="polite">
        <Loader2Icon className="size-5 animate-spin" aria-hidden />
        <div>
          <strong>Creating your bot</strong>
          <span>This usually takes a few seconds to start.</span>
        </div>
      </div>
    );
  }

  return (
    <form
      action="/dashboard/agent-creation-requests"
      method="post"
      className="grid gap-4"
      onSubmit={(event) => {
        if (submittedRef.current) {
          event.preventDefault();
          return;
        }
        submittedRef.current = true;
        window.setTimeout(() => setSubmitting(true), 0);
      }}
    >
      <input type="hidden" name="idempotencyKey" value={idempotencyKey} />
      {error ? (
        <p className="rounded-[var(--radius-card-inner)] border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}
      <div className="grid gap-2">
        <Label htmlFor="coreAgentDisplayName">Agent name</Label>
        <Input
          id="coreAgentDisplayName"
          name="displayName"
          placeholder="Oslo Agent"
          required
        />
      </div>
      <FormActionButton className="w-fit" pendingLabel="Creating...">
        <PlusIcon />
        Create agent
      </FormActionButton>
    </form>
  );
}
