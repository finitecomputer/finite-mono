"use client";

import * as React from "react";
import { useActionState, useState } from "react";
import {
  CheckIcon,
  CopyIcon,
  KeyRoundIcon,
  RotateCcwIcon,
  TriangleAlertIcon,
} from "lucide-react";

import {
  adminOpsIssueFriendKeyAction,
  adminOpsRotateKeyAction,
} from "@/app/actions";
import { FormActionButton } from "@/components/form-action-button";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  oneTimeKeyDisplay,
  oneTimeKeyError,
  type OneTimeKeyActionState,
} from "@/lib/admin-ops";

const IDLE_STATE: OneTimeKeyActionState = { status: "idle" };

/** Submit button that asks for confirmation before running a server action. */
export function ConfirmSubmitButton({
  confirmMessage,
  children,
  ...props
}: React.ComponentProps<typeof FormActionButton> & { confirmMessage: string }) {
  return (
    <FormActionButton
      {...props}
      onClick={(event) => {
        if (!window.confirm(confirmMessage)) {
          event.preventDefault();
        }
      }}
    >
      {children}
    </FormActionButton>
  );
}

function CopyRawKeyButton({ rawKey }: { rawKey: string }) {
  const [copied, setCopied] = useState(false);

  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      onClick={async () => {
        await navigator.clipboard.writeText(rawKey);
        setCopied(true);
        window.setTimeout(() => setCopied(false), 2000);
      }}
    >
      {copied ? <CheckIcon /> : <CopyIcon />}
      {copied ? "Copied" : "Copy key"}
    </Button>
  );
}

function OneTimeKeyPanel({ state }: { state: OneTimeKeyActionState }) {
  const display = oneTimeKeyDisplay(state);
  const error = oneTimeKeyError(state);

  if (error) {
    return (
      <p className="rounded-[var(--radius-card-inner)] border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
        {error}
      </p>
    );
  }
  if (!display) {
    return null;
  }

  return (
    <div className="grid gap-2 rounded-[var(--radius-card-inner)] border border-amber-500/40 bg-amber-500/10 p-3">
      <div className="flex items-center gap-2 text-sm font-semibold text-foreground">
        <TriangleAlertIcon className="size-4" aria-hidden />
        {display.warning}
      </div>
      <code className="break-all rounded bg-black/20 px-2 py-1 font-mono text-sm text-foreground">
        {display.rawKey}
      </code>
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <CopyRawKeyButton rawKey={display.rawKey} />
        <span className="font-mono">key {display.keyId}</span>
        {display.grantId ? <span className="font-mono">grant {display.grantId}</span> : null}
      </div>
    </div>
  );
}

/** Friend-key issue form with one-time raw-key display. */
export function AdminFriendKeyIssueForm() {
  const [state, formAction] = useActionState(adminOpsIssueFriendKeyAction, IDLE_STATE);

  return (
    <form
      action={formAction}
      className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4"
    >
      <div className="flex items-center gap-2 font-semibold text-foreground">
        <KeyRoundIcon className="size-4" />
        Issue friend key
      </div>
      <p className="text-sm text-muted-foreground">
        Approves a Finite Private grant for the email and issues a fresh key.
        The raw key is shown once, below.
      </p>
      <div className="grid gap-2">
        <Label htmlFor="adminFriendKeyEmail">Friend email</Label>
        <Input id="adminFriendKeyEmail" name="email" type="email" required />
      </div>
      <div className="grid gap-2">
        <Label htmlFor="adminFriendKeyLimitProfile">Limit profile (optional)</Label>
        <Input
          id="adminFriendKeyLimitProfile"
          name="limitProfileId"
          placeholder="finite-private-generous"
        />
      </div>
      <FormActionButton className="w-fit" pendingLabel="Issuing...">
        <KeyRoundIcon />
        Issue key
      </FormActionButton>
      <OneTimeKeyPanel state={state} />
    </form>
  );
}

/** Per-key rotate form; Core generates the replacement raw key and returns it once. */
export function AdminRotateKeyForm({ keyId }: { keyId: string }) {
  const [state, formAction] = useActionState(adminOpsRotateKeyAction, IDLE_STATE);

  return (
    <form action={formAction} className="grid gap-2">
      <input type="hidden" name="keyId" value={keyId} />
      <ConfirmSubmitButton
        variant="outline"
        size="sm"
        className="w-fit"
        pendingLabel="Rotating..."
        confirmMessage="Rotate this Finite Private key? The current raw key stops working immediately."
      >
        <RotateCcwIcon />
        Rotate
      </ConfirmSubmitButton>
      <OneTimeKeyPanel state={state} />
    </form>
  );
}
