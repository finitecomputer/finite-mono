"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { BrainIcon, CheckIcon, Loader2Icon, XIcon } from "lucide-react";

import {
  approveResponseMetadata,
  type BrainApproveChoice,
  type BrainApproveEnvelope,
} from "@/lib/brain-approval-metadata";
import { cn } from "@/lib/utils";

/// Chat-surface Brain cards, anchored to the messages that carry them.
///
/// An approval question arrives as the agent's chat message with
/// `metadata.approve` naming the server-held request; the card renders in
/// stream position under that message. Approving or dismissing executes
/// through the dashboard routes (hosted-device signing — the same authority
/// `fbrain approvals approve` exercises) and then sends the user's reply
/// carrying the same metadata convention, so the conversation keeps the
/// durable record. Card contents come from the Brain server's own request
/// state, never from the message prose.
///
/// No polling: new questions appear when their message streams in; the
/// refresh that follows an action re-reads server truth once.

export type BrainApprovalRequestDetail = {
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

type CardState = "idle" | "working" | "done" | "error";

function approvalLabel(detail: BrainApprovalRequestDetail | undefined, reference: { brainId: string }) {
  const action = detail?.action;
  if (action === "invite-commit") return "Invitation approval";
  if (action === "delegation-grant") return "Admin delegation";
  if (action) return action;
  return `Brain approval (${reference.brainId})`;
}

/// Server truth for the requests the conversation references. Fetched when
/// the referenced set changes — the messages themselves are the change signal.
export function useBrainApprovalDetails(requestIds: string[]) {
  const [details, setDetails] = useState<Map<string, BrainApprovalRequestDetail>>(new Map());
  const key = useMemo(() => requestIds.slice().sort().join(","), [requestIds]);
  useEffect(() => {
    if (!key) return;
    let cancelled = false;
    fetch("/api/brain/approvals", { cache: "no-store" })
      .then(async (response) => {
        if (!response.ok) return;
        const body = await response.json();
        const listed = Array.isArray(body.approvals) ? body.approvals : [];
        const map = new Map<string, BrainApprovalRequestDetail>();
        for (const detail of listed as BrainApprovalRequestDetail[]) {
          if (detail?.id) map.set(detail.id, detail);
        }
        if (!cancelled) setDetails(map);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [key]);
  return details;
}

export function BrainApprovalCards({
  message,
  envelope,
  details,
  resolution,
  onSendChoice,
}: {
  /// The agent delivery the cards anchor to (`is_mine` gates rendering).
  message: { is_mine?: boolean };
  /// Its decoded `metadata.approve` envelope — decoded once per message by
  /// the caller's memo, never parsed again here.
  envelope: BrainApproveEnvelope | null;
  /// Server-side request state by request id (from useBrainApprovalDetails).
  details: Map<string, BrainApprovalRequestDetail>;
  /// The user's recorded choice by request id, from their reply messages.
  resolution: Map<string, BrainApproveChoice>;
  /// Sends the user's reply with the approve response metadata attached.
  onSendChoice?: (text: string, metadataJson: string) => Promise<void>;
}) {
  const question = envelope?.question ?? null;
  if (!question || message.is_mine) return null;
  return (
    <section className="finite-brain-cards" aria-label="Brain actions">
      {question.requests.map((reference) => (
        <BrainApprovalCard
          key={`approval:${reference.requestId}`}
          reference={reference}
          detail={details.get(reference.requestId)}
          choice={resolution.get(reference.requestId)}
          onSendChoice={onSendChoice}
        />
      ))}
    </section>
  );
}

function BrainApprovalCard({
  reference,
  detail,
  choice,
  onSendChoice,
}: {
  reference: { brainId: string; requestId: string };
  detail: BrainApprovalRequestDetail | undefined;
  choice: BrainApproveChoice | undefined;
  onSendChoice?: (text: string, metadataJson: string) => Promise<void>;
}) {
  const [state, setState] = useState<CardState>("idle");
  const [error, setError] = useState("");

  const label = approvalLabel(detail, reference);
  const brainName = detail?.brainName ?? reference.brainId;
  const resolved = choice !== undefined;
  const expired = !resolved && detail !== undefined && detail.expiresAt * 1000 <= Date.now();
  const unavailable = !resolved && detail === undefined;
  // A request the server no longer lists as pending is either resolved out of
  // band (the CLI) or expired beyond the list window; both are closed states.
  const closed = resolved || expired || unavailable;

  const act = useCallback(
    async (
      path: string,
      body: unknown,
      choiceToSend: BrainApproveChoice,
      choiceText: string
    ) => {
      setState("working");
      setError("");
      try {
        const response = await fetch(path, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body),
        });
        if (!response.ok) {
          const failure = await response.json().catch(() => ({}));
          throw new Error(failure.error ?? `HTTP ${response.status}`);
        }
        const result = await response.json().catch(() => ({}));
        if (onSendChoice) {
          const artifactId =
            typeof result.approvalEventId === "string" ? result.approvalEventId : undefined;
          try {
            await onSendChoice(
              choiceText,
              approveResponseMetadata(choiceToSend, [reference], artifactId)
            );
          } catch {
            // The action executed server-side; the in-band record is
            // best-effort, exactly like any other chat send.
          }
        }
        setState("done");
      } catch (caught) {
        setState("error");
        setError(caught instanceof Error ? caught.message : "failed");
      }
    },
    [onSendChoice, reference]
  );

  return (
    <article
      className="finite-brain-card"
      data-kind="approval"
      data-resolved={resolved || undefined}
      data-expired={expired || undefined}
    >
      <header className="finite-brain-card__head">
        <BrainIcon aria-hidden className="size-4" />
        <strong>{label}</strong>
        <span className="finite-brain-card__brain">{brainName}</span>
      </header>
      {resolved ? (
        <p className="finite-brain-card__body">
          {choice === "approved"
            ? "You approved this action."
            : "You dismissed this request."}
        </p>
      ) : expired ? (
        <p className="finite-brain-card__body">
          This request expired. Ask your agent to file it again.
        </p>
      ) : unavailable ? (
        <p className="finite-brain-card__body">
          This request is no longer pending. Ask your agent if it still matters.
        </p>
      ) : (
        <p className="finite-brain-card__body">
          Your agent requested this action. Approving signs it with your account key.
        </p>
      )}
      {state === "error" ? (
        <p className="finite-brain-card__error" role="alert">
          {error || "The approval failed."}
        </p>
      ) : null}
      {!closed ? (
        <footer className="finite-brain-card__actions">
          <button
            type="button"
            disabled={state === "working" || state === "done"}
            onClick={() =>
              act(
                "/api/brain/approvals/approve",
                {
                  brainId: reference.brainId,
                  requestId: reference.requestId,
                  payload: detail?.payload ?? null,
                },
                "approved",
                `Approved: ${label.toLowerCase()} for ${brainName}`
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
            disabled={state === "working" || state === "done"}
            onClick={() =>
              act(
                "/api/brain/approvals/deny",
                { brainId: reference.brainId, requestId: reference.requestId },
                "denied",
                `Dismissed: ${label.toLowerCase()} for ${brainName}`
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
      ) : null}
    </article>
  );
}

type InvitationCard = {
  id: string;
  brainId?: string;
  inviteCode?: string;
  status?: string;
  ref?: string;
};

/// Pending Brain invitations for this account. Unlike approval questions,
/// invitations arrive out of band (someone invited you), so there is no chat
/// message to anchor to; the section refreshes on conversation updates.
export function BrainInvitationCards({
  className,
  revision = 0,
  onSendMessage,
}: {
  className?: string;
  revision?: number;
  onSendMessage?: (text: string) => Promise<void>;
}) {
  const [invitations, setInvitations] = useState<InvitationCard[]>([]);
  const [cardState, setCardState] = useState<Record<string, CardState>>({});
  const [cardError, setCardError] = useState<Record<string, string>>({});

  const refresh = useCallback(async () => {
    try {
      const response = await fetch("/api/brain/invitations", { cache: "no-store" });
      if (response.ok) {
        const body = await response.json();
        const pending = Array.isArray(body.invitations)
          ? body.invitations.filter(
              (invitation: InvitationCard) => invitation.status === "pending"
            )
          : [];
        setInvitations(pending.filter((invitation: InvitationCard) => Boolean(invitation.inviteCode)));
      }
    } catch {
      // Unavailable invitations are not an error surface; joining retries.
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh, revision]);

  const act = useCallback(
    async (key: string, path: string, body: unknown, choiceMessage: string) => {
      setCardState((state) => ({ ...state, [key]: "working" }));
      setCardError((errors) => ({ ...errors, [key]: "" }));
      try {
        const response = await fetch(path, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body),
        });
        if (!response.ok) {
          const failure = await response.json().catch(() => ({}));
          throw new Error(failure.error ?? `HTTP ${response.status}`);
        }
        if (onSendMessage) {
          try {
            await onSendMessage(choiceMessage);
          } catch {
            // Joining already succeeded; the chat note is best-effort.
          }
        }
        setCardState((state) => ({ ...state, [key]: "done" }));
        void refresh();
      } catch (caught) {
        setCardState((state) => ({ ...state, [key]: "error" }));
        setCardError((errors) => ({
          ...errors,
          [key]: caught instanceof Error ? caught.message : "failed",
        }));
      }
    },
    [onSendMessage, refresh]
  );

  if (invitations.length === 0) return null;

  return (
    <section className={cn("finite-brain-cards", className)} aria-label="Brain invitations">
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
              You were invited to a Brain. Joining adds your account as a member.
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
