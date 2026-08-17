"use client";

import { useCallback, useEffect, useState } from "react";
import { BrainIcon, CheckIcon, Loader2Icon, XIcon } from "lucide-react";

import { cn } from "@/lib/utils";

/// Chat-surface Brain cards: pending approval requests the account's human
/// principal can sign, and pending invitations they can join. The server
/// routes drive the hosted chat device's signing ops — the same authority
/// `fbrain approvals approve` exercises.

type ApprovalCard = {
  id: string;
  brainId: string;
  brainName: string;
  action: string;
  requestedByNpub: string;
  expiresAt: number;
  createdAt: string;
  payload: {
    action?: string;
    planId?: string | null;
    nonce?: string;
    expiresAt?: number;
  } | null;
};

type InvitationCard = {
  id: string;
  brainId?: string;
  inviteCode?: string;
  status?: string;
  ref?: string;
};

type CardState = "idle" | "working" | "done" | "error";

function approvalLabel(card: ApprovalCard) {
  if (card.action === "invite-commit") return "Invitation approval";
  if (card.action === "delegation-grant") return "Admin delegation";
  return card.action;
}

export function BrainActionCards({
  className,
  onSendMessage,
  revision = 0,
}: {
  className?: string;
  /// Sends the user's choice into the conversation stream as their own
  /// message — the approval's durable in-band record.
  onSendMessage?: (text: string) => Promise<void>;
  /// Changes when the conversation updates (new messages); cards refetch
  /// so a freshly filed request appears without a manual page refresh.
  revision?: number;
}) {
  const [approvals, setApprovals] = useState<ApprovalCard[]>([]);
  const [invitations, setInvitations] = useState<InvitationCard[]>([]);
  const [unavailable, setUnavailable] = useState(false);
  const [cardState, setCardState] = useState<Record<string, CardState>>({});
  const [cardError, setCardError] = useState<Record<string, string>>({});

  const refresh = useCallback(async () => {
    try {
      const [approvalsResponse, invitationsResponse] = await Promise.all([
        fetch("/api/brain/approvals", { cache: "no-store" }),
        fetch("/api/brain/invitations", { cache: "no-store" }),
      ]);
      if (approvalsResponse.ok) {
        const body = await approvalsResponse.json();
        const now = Date.now();
        const pending = (
          Array.isArray(body.approvals) ? body.approvals : []
        ) as ApprovalCard[];
        setApprovals(
          pending
            .filter((card) => card.expiresAt * 1000 > now)
            .sort((a, b) => b.createdAt.localeCompare(a.createdAt))
        );
      } else if (approvalsResponse.status !== 401 && approvalsResponse.status !== 503) {
        setUnavailable(true);
      }
      if (invitationsResponse.ok) {
        const body = await invitationsResponse.json();
        const pending = Array.isArray(body.invitations)
          ? body.invitations.filter(
              (invitation: InvitationCard) => invitation.status === "pending"
            )
          : [];
        setInvitations(
          pending.filter((invitation: InvitationCard) => Boolean(invitation.inviteCode))
        );
      }
    } catch {
      setUnavailable(true);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh, revision]);

  // A filing can happen with no chat activity in this tab; poll lightly
  // while cards are mounted so requests stream in on their own.
  useEffect(() => {
    const timer = setInterval(() => void refresh(), 20_000);
    return () => clearInterval(timer);
  }, [refresh]);

  const act = useCallback(
    async (
      key: string,
      path: string,
      body: unknown,
      onDone: () => void,
      choiceMessage?: string
    ) => {
      setCardState((state) => ({ ...state, [key]: "working" }));
      setCardError((errors) => ({ ...errors, [key]: "" }));
      try {
        const response = await fetch(path, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body),
        });
        if (!response.ok) {
          const error = await response.json().catch(() => ({}));
          throw new Error(error.error ?? `HTTP ${response.status}`);
        }
        if (choiceMessage && onSendMessage) {
          try {
            await onSendMessage(choiceMessage);
          } catch {
            // The action executed; the conversation record is best-effort
            // until message metadata carries the artifact natively.
          }
        }
        setCardState((state) => ({ ...state, [key]: "done" }));
        onDone();
      } catch (error) {
        setCardState((state) => ({ ...state, [key]: "error" }));
        setCardError((errors) => ({
          ...errors,
          [key]: error instanceof Error ? error.message : "failed",
        }));
      }
    },
    [onSendMessage]
  );

  if (
    approvals.length === 0 &&
    invitations.length === 0 &&
    !unavailable
  ) {
    return null;
  }

  return (
    <section
      className={cn("finite-brain-cards", className)}
      aria-label="Brain actions"
    >
      {unavailable ? (
        <p className="finite-brain-cards__note">
          Brain actions are unavailable right now.
        </p>
      ) : null}
      {approvals.map((card) => {
        const key = `approval:${card.id}`;
        const state = cardState[key] ?? "idle";
        const expired = card.expiresAt * 1000 <= Date.now();
        return (
          <article
            key={key}
            className="finite-brain-card"
            data-kind="approval"
            data-expired={expired || undefined}
          >
            <header className="finite-brain-card__head">
              <BrainIcon aria-hidden className="size-4" />
              <strong>{approvalLabel(card)}</strong>
              <span className="finite-brain-card__brain">{card.brainName}</span>
            </header>
            <p className="finite-brain-card__body">
              {expired
                ? "This request expired. Ask your agent to file it again."
                : "Your agent requested this action. Approving signs it with your account key."}
            </p>
            {state === "error" ? (
              <p className="finite-brain-card__error" role="alert">
                {cardError[key] || "The approval failed."}
              </p>
            ) : null}
            <footer className="finite-brain-card__actions">
              <button
                type="button"
                disabled={state === "working" || state === "done" || expired}
                onClick={() =>
                  act(
                    key,
                    "/api/brain/approvals/approve",
                    {
                      brainId: card.brainId,
                      requestId: card.id,
                      payload: card.payload,
                    },
                    () => void refresh(),
                    `Approved: ${approvalLabel(card).toLowerCase()} for ${card.brainName}`
                  )
                }
              >
                {state === "working" ? (
                  <Loader2Icon aria-hidden className="size-4 animate-spin" />
                ) : state === "done" ? (
                  <CheckIcon aria-hidden className="size-4" />
                ) : null}
                Approve
              </button>
              <button
                type="button"
                disabled={state === "working" || state === "done" || expired}
                onClick={() =>
                  act(
                    key,
                    "/api/brain/approvals/deny",
                    { brainId: card.brainId, requestId: card.id },
                    () => void refresh(),
                    `Dismissed: ${approvalLabel(card).toLowerCase()} for ${card.brainName}`
                  )
                }
              >
                {state === "working" ? (
                  <Loader2Icon aria-hidden className="size-4 animate-spin" />
                ) : state === "done" ? (
                  <XIcon aria-hidden className="size-4" />
                ) : null}
                Dismiss
              </button>
            </footer>
          </article>
        );
      })}
      {invitations.map((card) => {
        const key = `invitation:${card.id}`;
        const state = cardState[key] ?? "idle";
        return (
          <article key={key} className="finite-brain-card" data-kind="invitation">
            <header className="finite-brain-card__head">
              <BrainIcon aria-hidden className="size-4" />
              <strong>Brain invitation</strong>
              <span className="finite-brain-card__brain">{card.ref ?? card.brainId ?? ""}</span>
            </header>
            <p className="finite-brain-card__body">
              You were invited to a Brain. Joining adds your account as a
              member.
            </p>
            {state === "error" ? (
              <p className="finite-brain-card__error" role="alert">
                {cardError[key] || "Joining failed."}
              </p>
            ) : null}
            <footer className="finite-brain-card__actions">
              <button
                type="button"
                disabled={state === "working" || state === "done"}
                onClick={() =>
                  act(
                    key,
                    "/api/brain/invitations/accept",
                    { inviteCode: card.inviteCode },
                    () => void refresh(),
                    `Joined ${card.ref ?? card.brainId ?? "a Brain"}`
                  )
                }
              >
                {state === "working" ? (
                  <Loader2Icon aria-hidden className="size-4 animate-spin" />
                ) : state === "done" ? (
                  <CheckIcon aria-hidden className="size-4" />
                ) : null}
                Join Brain
              </button>
            </footer>
          </article>
        );
      })}
    </section>
  );
}
