"use client";

import { useRef, useState, type FormEvent } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { supportEmailHref } from "@/lib/support-contact";

type Receipt = { id: string; status: "pending" | "sent" | "failed" };

export function SupportContact({ email }: { email: string | null }) {
  return email ? (
    <span>Need help? Email <a className="underline" href={supportEmailHref(email)}>{email}</a>.</span>
  ) : <span>Need help? Contact your system administrator.</span>;
}

export function SupportReport({ open, onOpenChange, initialMessage, requestId, projectId, machineLabel, email, replyTo, onAccepted }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialMessage: string;
  requestId: string;
  projectId: string | null;
  machineLabel: string;
  email: string | null;
  replyTo: string | null;
  onAccepted: () => void;
}) {
  const [message, setMessage] = useState(initialMessage);
  const [busy, setBusy] = useState(false);
  const [attempted, setAttempted] = useState(false);
  const [receipt, setReceipt] = useState<Receipt | null>(null);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);
  const payload = useRef<string | null>(null);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (inFlight.current || receipt || !email || !replyTo || !message.trim()) return;
    inFlight.current = true;
    setBusy(true);
    setError(null);
    setAttempted(true);
    // Preserve the exact reviewed payload and key after an ambiguous network failure.
    payload.current ??= JSON.stringify({ idempotencyKey: requestId, projectId, message: message.trim(), supportEmail: email, replyTo });
    try {
      const response = await fetch("/api/support", {
        method: "POST", headers: { "content-type": "application/json" }, body: payload.current,
      });
      const result: unknown = await response.json();
      if (!response.ok) {
        const reason = result && typeof result === "object" && "error" in result && typeof result.error === "string"
          ? result.error : "Support is unavailable. Please email directly.";
        throw new Error(reason);
      }
      if (!result || typeof result !== "object" || !("id" in result) || typeof result.id !== "string"
        || !("status" in result) || !["pending", "sent", "failed"].includes(String(result.status))) {
        throw new Error("Could not confirm receipt. Retry this report or email directly.");
      }
      setReceipt(result as Receipt);
      onAccepted();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not confirm receipt. Retry or email directly.");
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => { if (!busy) onOpenChange(next); }}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Contact support</DialogTitle>
          <DialogDescription>
            {email ? <>Send a report to {email}. Replies are handled by email.</> : "Your administrator has not configured a support email."}
          </DialogDescription>
        </DialogHeader>
        {receipt ? (
          <div role="status" className="space-y-2">
            <p>{receipt.status === "sent" ? "Your report was sent." : receipt.status === "failed" ? "Email delivery could not be confirmed. Please email support directly and include this reference." : "Your report is saved for email delivery."}</p>
            <p>Reference: {receipt.id}</p>
            <p>Reply email: {replyTo}</p>
            <SupportContact email={email} />
          </div>
        ) : (
          <form onSubmit={submit} className="space-y-4">
            <p>Agent: {machineLabel}</p>
            {projectId ? <p className="text-xs text-muted-foreground">Agent reference: {projectId}</p> : null}
            <p>Reply email: {replyTo ?? "Sign in to submit a report."}</p>
            <label className="grid gap-2" htmlFor="support-message">
              What happened?
              <textarea id="support-message" className="min-h-36 w-full rounded-md border bg-transparent p-3" maxLength={4000} required value={message} disabled={attempted} onChange={(event) => setMessage(event.target.value)} />
            </label>
            <p className="text-sm text-muted-foreground">Only this report, your reply email, and the agent reference are shared. Conversation history, files, and logs are not attached.</p>
            {error ? <p role="alert">{error}</p> : null}
            <SupportContact email={email} />
            <DialogFooter>
              <Button type="button" variant="outline" disabled={busy} onClick={() => onOpenChange(false)}>Close</Button>
              <Button type="submit" disabled={busy || !email || !replyTo || !message.trim() || message.length > 4000}>
                {busy ? "Sending…" : attempted ? "Retry report" : "Send report"}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
