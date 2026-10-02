"use client";

import { useState } from "react";
import { ShieldCheckIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export const ADMISSION_REQUEST_TIMEOUT_MS = 30_000;
export const ADMISSION_TIMEOUT_MESSAGE =
  "The admission change is taking too long. Try again.";
const ADMISSION_ACCOUNT_ID_PATTERN = /^[0-9a-f]{64}$/u;

export function parseAdmissionAccountId(value: string): string | null {
  const trimmed = value.trim().toLowerCase();
  return ADMISSION_ACCOUNT_ID_PATTERN.test(trimmed) ? trimmed : null;
}

export function admissionErrorMessage(error: unknown): string {
  if (
    error instanceof Error &&
    (error.name === "TimeoutError" || error.name === "AbortError")
  ) {
    return ADMISSION_TIMEOUT_MESSAGE;
  }
  return error instanceof Error && error.message.trim()
    ? error.message
    : "Chat admission is unavailable right now.";
}

export function admissionSuccessText(
  status: "applied" | "sent",
  action: "grant" | "revoke",
  agentName: string
): string {
  if (status === "applied") {
    return `${
      action === "grant" ? "Granted" : "Revoked"
    } chat access. It takes effect at ${agentName}'s next gateway restart.`;
  }
  // The sidecar applies admission commands without a receipt, so "sent" is
  // the honest report: the dashboard cannot observe the apply or a refusal.
  return (
    `${action === "grant" ? "Granted" : "Revoked"} chat access — command sent. ` +
    `${agentName} applies admission changes without a confirmation reply; the ` +
    "change takes effect at its next gateway restart."
  );
}

type AdmissionNotice = { kind: "success" | "error"; text: string };

export function AgentAdmissionPanel({
  machineId,
  agentName,
}: {
  machineId: string;
  agentName: string;
}) {
  const [accountId, setAccountId] = useState("");
  const [busy, setBusy] = useState<"grant" | "revoke" | null>(null);
  const [notice, setNotice] = useState<AdmissionNotice | null>(null);
  const endpoint = `/api/connections/machines/${encodeURIComponent(machineId)}/admission`;

  async function submit(action: "grant" | "revoke") {
    const normalized = parseAdmissionAccountId(accountId);
    if (!normalized) {
      setNotice({
        kind: "error",
        text: "Enter the account id as 64 hexadecimal characters.",
      });
      return;
    }
    setBusy(action);
    setNotice(null);
    try {
      const response = await fetch(endpoint, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ action, accountId: normalized }),
        cache: "no-store",
        signal: AbortSignal.timeout(ADMISSION_REQUEST_TIMEOUT_MS),
      });
      const payload = (await response.json().catch(() => ({}))) as {
        status?: "applied" | "sent";
        error?: unknown;
      };
      if (!response.ok) {
        throw new Error(
          typeof payload.error === "string" && payload.error.trim()
            ? payload.error
            : "Chat admission is unavailable right now."
        );
      }
      setNotice({
        kind: "success",
        text: admissionSuccessText(payload.status === "applied" ? "applied" : "sent", action, agentName),
      });
      setAccountId("");
    } catch (error) {
      setNotice({ kind: "error", text: admissionErrorMessage(error) });
    } finally {
      setBusy(null);
    }
  }

  return (
    <section className="rounded-xl border bg-card p-5">
      <div className="flex items-start gap-3">
        <ShieldCheckIcon className="mt-0.5 size-5 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <h2 className="font-semibold">Chat admission</h2>
          <p className="mt-1 max-w-2xl text-sm leading-6 text-muted-foreground">
            Choose which accounts may start chats with {agentName}. Only allowlisted
            accounts get past the agent&apos;s Welcome check.
          </p>
          <div className="mt-4 flex flex-wrap items-center gap-2">
            <Input
              value={accountId}
              onChange={(event) => setAccountId(event.target.value)}
              spellCheck={false}
              autoComplete="off"
              maxLength={64}
              placeholder="Account id (64 hex characters)"
              aria-label="Account id to grant or revoke chat admission"
              className="w-80 font-mono"
            />
            <Button
              type="button"
              disabled={busy !== null}
              onClick={() => void submit("grant")}
            >
              {busy === "grant" ? "Granting…" : "Grant"}
            </Button>
            <Button
              type="button"
              variant="destructive"
              disabled={busy !== null}
              onClick={() => void submit("revoke")}
            >
              {busy === "revoke" ? "Revoking…" : "Revoke"}
            </Button>
          </div>
          <p className="mt-2 text-xs leading-5 text-muted-foreground">
            The agent applies admission changes without a confirmation reply, and they
            take effect at its next gateway restart. The current allowlist lives on
            the agent, so it cannot be listed here yet.
          </p>
          {notice ? (
            <p
              role={notice.kind === "error" ? "alert" : "status"}
              className={
                notice.kind === "error"
                  ? "mt-3 text-sm text-destructive"
                  : "mt-3 text-sm text-muted-foreground"
              }
            >
              {notice.text}
            </p>
          ) : null}
        </div>
      </div>
    </section>
  );
}
