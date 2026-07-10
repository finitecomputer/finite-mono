"use client";

import * as React from "react";
import { useActionState, useState } from "react";
import {
  CheckIcon,
  CopyIcon,
  DownloadIcon,
  KeyRoundIcon,
  RotateCcwIcon,
  TriangleAlertIcon,
} from "lucide-react";

import {
  adminOpsIssueLaunchCodeBatchAction,
  adminOpsIssueFriendKeyAction,
  adminOpsRotateKeyAction,
} from "@/app/actions";
import { FormActionButton } from "@/components/form-action-button";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  launchCodeDownloadFilename,
  launchCodeDownloadText,
  oneTimeKeyDisplay,
  oneTimeKeyError,
  type OneTimeKeyActionState,
  type OneTimeLaunchCodeActionState,
} from "@/lib/admin-ops";

const IDLE_STATE: OneTimeKeyActionState = { status: "idle" };
const IDLE_LAUNCH_CODE_STATE: OneTimeLaunchCodeActionState = { status: "idle" };

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

/** Operator-only form that holds plaintext Launch Codes only in client action state. */
export function AdminLaunchCodeBatchIssueForm() {
  const [state, formAction] = useActionState(
    adminOpsIssueLaunchCodeBatchAction,
    IDLE_LAUNCH_CODE_STATE
  );
  const [expiryHours, setExpiryHours] = useState("168");

  return (
    <form
      action={formAction}
      className="grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4"
    >
      <div className="flex items-center gap-2 font-semibold text-foreground">
        <KeyRoundIcon className="size-4" />
        Issue Launch Code batch
      </div>
      <p className="text-sm text-muted-foreground">
        Choose a named, exact-size batch. Codes are shown once after issuance and are never available in later views.
      </p>
      <div className="grid gap-3 md:grid-cols-3">
        <div className="grid gap-2 md:col-span-1">
          <Label htmlFor="launchCodeBatchName">Batch name</Label>
          <Input id="launchCodeBatchName" name="name" maxLength={120} required placeholder="July training" />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="launchCodeBatchCount">Exact code count</Label>
          <Input id="launchCodeBatchCount" name="codeCount" type="number" min={1} max={1000} defaultValue={1} required />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="launchCodeBatchExpiry">Expiry (hours)</Label>
          <Input
            id="launchCodeBatchExpiry"
            name="expiresInHours"
            type="number"
            min={1}
            max={720}
            required
            value={expiryHours}
            onChange={(event) => setExpiryHours(event.target.value)}
          />
          <div className="flex flex-wrap gap-2 text-xs">
            <button type="button" className="underline" onClick={() => setExpiryHours("24")}>24 hours</button>
            <button type="button" className="underline" onClick={() => setExpiryHours("168")}>7 days</button>
          </div>
        </div>
      </div>
      <p className="text-xs text-muted-foreground">Maximum expiry is 30 days. Use 1 code for the canary or 12 for a training group.</p>
      <FormActionButton className="w-fit" pendingLabel="Issuing...">
        <KeyRoundIcon />
        Issue codes
      </FormActionButton>
      <OneTimeLaunchCodePanel state={state} />
    </form>
  );
}

function OneTimeLaunchCodePanel({ state }: { state: OneTimeLaunchCodeActionState }) {
  if (state.status === "error") {
    return (
      <p className="rounded-[var(--radius-card-inner)] border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
        {state.error}
      </p>
    );
  }
  if (state.status !== "issued") {
    return null;
  }
  const text = launchCodeDownloadText(state.codes);

  return (
    <div className="grid gap-3 rounded-[var(--radius-card-inner)] border border-amber-500/40 bg-amber-500/10 p-3">
      <div className="flex items-center gap-2 text-sm font-semibold text-foreground">
        <TriangleAlertIcon className="size-4" aria-hidden />
        These {state.codes.length} Launch Code{state.codes.length === 1 ? "" : "s"} are shown once. Copy or download them now.
      </div>
      <textarea
        readOnly
        value={text}
        aria-label="Issued Launch Codes"
        className="min-h-28 w-full rounded bg-black/20 p-2 font-mono text-sm text-foreground"
      />
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <Button type="button" variant="outline" size="sm" onClick={() => void copyLaunchCodes(text)}>
          <CopyIcon />
          Copy codes
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => downloadLaunchCodes(text, launchCodeDownloadFilename(state.batch.name))}
        >
          <DownloadIcon />
          Download codes
        </Button>
        <span>{state.batch.name} · expires {new Date(state.batch.expiresAt).toLocaleString()}</span>
      </div>
    </div>
  );
}

async function copyLaunchCodes(text: string) {
  await navigator.clipboard.writeText(text);
}

function downloadLaunchCodes(text: string, filename: string) {
  const url = URL.createObjectURL(new Blob([text], { type: "text/plain;charset=utf-8" }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
}
