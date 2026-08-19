"use client";

/**
 * Live brain:// document viewer (plan Phase 2, the proof slice).
 *
 * The browser generates an ephemeral nostr keypair in tab memory, mints a
 * viewer session through the dashboard (the hosted device signs as the
 * signed-in principal), waits for the agent to wrap the Folder Key to the
 * ephemeral key, then pulls ciphertext from the Brain server's
 * encrypted-read route, decrypts client-side, renders markdown, and follows
 * /brain-updates SSE for live deltas — no reload. Honest states throughout:
 * waiting for the agent, session expired, revoked, folder too large.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { RefreshCw, X } from "lucide-react";
import type { BrainDocRef } from "@/lib/brain-doc-url";
import {
  brainAuthHeader,
  findBrainDoc,
  generateViewerKey,
  replayViewerRecords,
  unwrapFolderKey,
  viewerNpub,
  type ViewerRecord,
} from "@/lib/brain-viewer-crypto";

type ViewerPhase =
  | "boot"
  | "waiting"
  | "loading"
  | "ready"
  | "expired"
  | "revoked"
  | "too-large"
  | "error";

type ViewerSession = {
  id?: string;
  status?: string;
  wrappedKeyPayload?: string;
  completedByNpub?: string;
  expiresAt?: string;
};

type RecordsResponse = {
  records?: ViewerRecord[];
  latestSequence?: number;
  sessionExpiresAt?: string;
};

const POLL_INTERVAL_MS = 2_000;
const RENEW_MARGIN_MS = 5 * 60 * 1_000;
const REQUESTED_TTL_SECS = 3_600;

function phaseText(phase: ViewerPhase, detail: string | null): string {
  switch (phase) {
    case "boot":
      return "Opening Brain document…";
    case "waiting":
      return "Waiting for your agent to hand over the folder key…";
    case "loading":
      return "Decrypting folder…";
    case "expired":
      return "Session expired — click the brain:// link again.";
    case "revoked":
      return "This viewer session was revoked.";
    case "too-large":
      return "This folder is too large for live view.";
    default:
      return detail ?? "This Brain document cannot be shown right now.";
  }
}

export function BrainDocPanel({
  className,
  doc,
  onClose,
}: {
  className: string;
  doc: BrainDocRef;
  onClose: () => void;
}) {
  const [phase, setPhase] = useState<ViewerPhase>("boot");
  const [detail, setDetail] = useState<string | null>(null);
  const [markdown, setMarkdown] = useState<string | null>(null);
  const [lastUpdated, setLastUpdated] = useState<number | null>(null);
  const [reloadNonce, setReloadNonce] = useState(0);
  // Tab memory only: the ephemeral key never leaves this component and is
  // never written to durable storage (asserted by the browser suite).
  const secretRef = useRef<Uint8Array | null>(null);
  const objectsRef = useRef(
    new Map<string, { objectId: string; path: string; markdown: string; revision: number }>(),
  );
  const lastSequenceRef = useRef(0);

  const renderTarget = useCallback(() => {
    const found = findBrainDoc(objectsRef.current, doc.path);
    if (!found) {
      setPhase("error");
      setDetail(`Document ${doc.path} was not found in this folder.`);
      return;
    }
    setMarkdown(found.markdown);
    setLastUpdated(Date.now());
    setPhase("ready");
  }, [doc.path]);

  useEffect(() => {
    let cancelled = false;
    const controller = new AbortController();
    objectsRef.current = new Map();
    lastSequenceRef.current = 0;
    setMarkdown(null);
    setDetail(null);
    setPhase("boot");

    const fail = (message: string) => {
      if (!cancelled) {
        setPhase("error");
        setDetail(message);
      }
    };

    async function signedFetch(
      secret: Uint8Array,
      origin: string,
      method: string,
      path: string,
    ): Promise<{ status: number; body: Record<string, unknown> }> {
      const url = `${origin}${path}`;
      const authorization = await brainAuthHeader(secret, method, url);
      const response = await fetch(url, {
        method,
        headers: { authorization, accept: "application/json" },
        signal: controller.signal,
        cache: "no-store",
      });
      let body: Record<string, unknown> = {};
      try {
        body = (await response.json()) as Record<string, unknown>;
      } catch {
        body = {};
      }
      return { status: response.status, body };
    }

    async function requestSession(secret: Uint8Array): Promise<ViewerSession | null> {
      const mint = await fetch("/api/brain/viewer-sessions", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          brainId: doc.brainId,
          folderId: doc.folderId,
          ephemeralNpub: viewerNpub(secret),
          requestedTtlSecs: REQUESTED_TTL_SECS,
        }),
        signal: controller.signal,
        cache: "no-store",
      });
      if (!mint.ok) {
        const payload = (await mint.json().catch(() => ({}))) as { error?: string };
        fail(payload.error ?? "The viewer session could not be created.");
        return null;
      }
      return (await mint.json()) as ViewerSession;
    }

    async function awaitReadySession(
      secret: Uint8Array,
      origin: string,
      session: ViewerSession,
    ): Promise<{ key: string; expiresAt?: string } | "expired" | "revoked" | null> {
      let current = session;
      for (;;) {
        if (cancelled) return null;
        if (
          current.status === "ready"
          && current.wrappedKeyPayload
          && current.completedByNpub
        ) {
          const key = await unwrapFolderKey(
            secret,
            current.completedByNpub,
            current.wrappedKeyPayload,
          );
          return { key, expiresAt: current.expiresAt };
        }
        if (current.status === "revoked") return "revoked";
        if (current.status === "expired") return "expired";
        if (!current.id) return null;
        const poll = await signedFetch(
          secret,
          origin,
          "GET",
          `/v1/viewer-session-requests/${current.id}`,
        );
        current = poll.body as ViewerSession;
        setPhase("waiting");
        await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
      }
    }

    async function fetchRecords(
      secret: Uint8Array,
      origin: string,
      key: string,
      after: number,
    ): Promise<boolean> {
      const { status, body } = await signedFetch(
        secret,
        origin,
        "GET",
        `/v1/brains/${doc.brainId}/folders/${doc.folderId}/records?after=${after}`,
      );
      if (status === 403) {
        const error = (body as { error?: string }).error ?? "";
        if (!cancelled) setPhase(error.includes("revoked") ? "revoked" : "expired");
        return false;
      }
      if (status === 413) {
        if (!cancelled) setPhase("too-large");
        return false;
      }
      if (status !== 200) {
        fail("The Brain server declined the encrypted read.");
        return false;
      }
      const records = ((body as RecordsResponse).records ?? []).slice().sort(
        (left, right) => (left.sequence ?? 0) - (right.sequence ?? 0),
      );
      if (records.length > 0) {
        setPhase("loading");
        const decrypted = await replayViewerRecords(
          records,
          key,
          doc.brainId,
          doc.folderId,
        );
        for (const [objectId, object] of decrypted) {
          objectsRef.current.set(objectId, object);
        }
        lastSequenceRef.current = records.reduce(
          (max, record) => Math.max(max, record.sequence ?? 0),
          lastSequenceRef.current,
        );
      }
      return true;
    }

    async function followUpdates(
      secret: Uint8Array,
      origin: string,
      key: string,
      renewBeforeMs: number,
    ) {
      // fetch-streamed SSE: EventSource cannot carry the NIP-98 header.
      const url = `${origin}/v1/brain-updates`;
      const authorization = await brainAuthHeader(secret, "GET", url);
      const response = await fetch(url, {
        headers: { authorization, accept: "text/event-stream" },
        signal: controller.signal,
        cache: "no-store",
      });
      if (!response.ok || !response.body) {
        fail("Live updates are unavailable; the document still renders.");
        return;
      }
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      let eventName = "";
      let renewed = false;
      for (;;) {
        if (cancelled) return;
        // Silent renew (grill #1): mint a fresh request before the TTL
        // lapses while the tab is open; the agent's completion extends the
        // session server-side and the active key keeps decrypting.
        if (
          !renewed
          && renewBeforeMs > RENEW_MARGIN_MS
          && Date.now() >= renewBeforeMs - RENEW_MARGIN_MS
        ) {
          renewed = true;
          void requestSession(secret).catch(() => undefined);
        }
        const { done, value } = await reader.read();
        if (done) return;
        buffer += decoder.decode(value, { stream: true });
        let boundary = buffer.indexOf("\n\n");
        while (boundary !== -1) {
          const frame = buffer.slice(0, boundary);
          buffer = buffer.slice(boundary + 2);
          for (const line of frame.split("\n")) {
            if (line.startsWith("event:")) {
              eventName = line.slice(6).trim();
            } else if (line.startsWith("data:") && eventName === "brain_update") {
              if (line.includes(`"brain_id":"${doc.brainId}"`)) {
                void fetchRecords(secret, origin, key, lastSequenceRef.current).then((ok) => {
                  if (ok && !cancelled) renderTarget();
                });
              }
              eventName = "";
            }
          }
          boundary = buffer.indexOf("\n\n");
        }
      }
    }

    void (async () => {
      secretRef.current ??= generateViewerKey();
      const secret = secretRef.current;
      const configResponse = await fetch("/api/brain/viewer/config", {
        signal: controller.signal,
        cache: "no-store",
      });
      if (!configResponse.ok) {
        fail("The Brain viewer is not configured on this deployment.");
        return;
      }
      const { origin } = (await configResponse.json()) as { origin: string };
      const session = await requestSession(secret);
      if (!session?.id || cancelled) return;
      const ready = await awaitReadySession(secret, origin, session);
      if (!ready || cancelled) return;
      if (ready === "revoked") {
        setPhase("revoked");
        return;
      }
      if (ready === "expired") {
        setPhase("expired");
        return;
      }
      const loaded = await fetchRecords(secret, origin, ready.key, 0);
      if (!loaded || cancelled) return;
      renderTarget();
      const expiresAtMs = ready.expiresAt ? Date.parse(ready.expiresAt) : Number.NaN;
      await followUpdates(
        secret,
        origin,
        ready.key,
        Number.isNaN(expiresAtMs) ? 0 : expiresAtMs - Date.now(),
      );
    })().catch((error: unknown) => {
      if (!cancelled) fail(error instanceof Error ? error.message : String(error));
    });

    return () => {
      cancelled = true;
      controller.abort();
    };
  }, [doc, renderTarget, reloadNonce]);

  return (
    <aside className={className} aria-label="Brain document preview">
      <div className="finite-chat__browser">
        <div className="finite-chat__browser-chrome">
          <span className="finite-chat__traffic-lights" aria-hidden><span /><span /><span /></span>
          <span
            className="finite-chat__site-switcher finite-chat__site-switcher--single"
            title={`brain://${doc.brainId}/${doc.folderId}/${doc.path}`}
          >
            brain://{doc.folderId}/{doc.path}
          </span>
          <input
            aria-label="Document URL"
            readOnly
            value={`brain://${doc.brainId}/${doc.folderId}/${doc.path}`}
          />
          <div className="finite-chat__browser-actions">
            <button
              type="button"
              aria-label="Reload document"
              onClick={() => setReloadNonce((value) => value + 1)}
            >
              <RefreshCw className="size-3.5" />
            </button>
            <button type="button" aria-label="Close preview" onClick={onClose}>
              <X className="size-3.5" />
            </button>
          </div>
        </div>
        <div className="finite-chat__browser-viewport">
          {phase === "ready" && markdown !== null ? (
            <div className="finite-chat__brain-doc-content" data-testid="brain-doc-content">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{markdown}</ReactMarkdown>
              {lastUpdated !== null ? (
                <p className="finite-chat__brain-doc-updated" data-testid="brain-doc-updated">
                  Live · updated {new Date(lastUpdated).toLocaleTimeString()}
                </p>
              ) : null}
            </div>
          ) : (
            <div className="finite-chat__brain-doc-status" data-testid="brain-doc-status">
              {phaseText(phase, detail)}
            </div>
          )}
        </div>
      </div>
    </aside>
  );
}
