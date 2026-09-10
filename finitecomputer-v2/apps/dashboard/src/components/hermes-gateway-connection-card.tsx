"use client";

/**
 * Spike: the "connect hermes desktop (or any tui_gateway client) to this
 * agent" card. Shows the gateway's public connection URL and credential with
 * copy actions, plus a one-click reachability probe (connect → session.list).
 *
 * Everything is env-driven, same family the chat provider reads:
 *   NEXT_PUBLIC_HERMES_GATEWAY_PUBLIC_URL  what clients should dial
 *                                          (prod shape: wss://<agent>.agents.finite.computer/api/ws)
 *   NEXT_PUBLIC_HERMES_GATEWAY_WS_URL      fallback for the display URL + probe target
 *   NEXT_PUBLIC_HERMES_GATEWAY_TOKEN       static-token credential, or
 *   NEXT_PUBLIC_HERMES_GATEWAY_USERNAME/PASSWORD  gated-mode credential
 *
 * In production this info comes from the agent's runtime config via Core
 * (per-agent URL + credential minted at spawn); env keeps the spike honest
 * about its single source.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { BotIcon, CheckIcon, CopyIcon, LoaderCircleIcon, PlugZapIcon } from "lucide-react";

import { ConnectionCard } from "@/components/connection-card";

const PUBLIC_URL =
  process.env.NEXT_PUBLIC_HERMES_GATEWAY_PUBLIC_URL
  || process.env.NEXT_PUBLIC_HERMES_GATEWAY_WS_URL
  || "";
const TOKEN = process.env.NEXT_PUBLIC_HERMES_GATEWAY_TOKEN || "";
const USERNAME = process.env.NEXT_PUBLIC_HERMES_GATEWAY_USERNAME || "";
const PASSWORD = process.env.NEXT_PUBLIC_HERMES_GATEWAY_PASSWORD || "";

export function HermesGatewayConnectionCard() {
  return (
    <ConnectionCard
      name="Hermes Gateway"
      state={PUBLIC_URL ? "connected" : "unavailable"}
      account={PUBLIC_URL ? null : "No gateway configured"}
      description="Connect hermes desktop to this agent over the tui_gateway protocol."
      icon={<BotIcon className="size-5" />}
    >
      {PUBLIC_URL ? (
        <div className="space-y-3">
          <CopyField label="Connection URL" value={PUBLIC_URL} />
          {TOKEN ? <CopyField label="Token" value={TOKEN} /> : null}
          {USERNAME ? (
            <>
              <CopyField label="Username" value={USERNAME} />
              {PASSWORD ? <CopyField label="Password" value={PASSWORD} sensitive /> : null}
            </>
          ) : null}
          <p className="text-xs text-[var(--text-secondary)]">
            hermes desktop → add connection → paste the URL
            {TOKEN ? " and token" : " and credentials"}. The gateway speaks the
            same protocol as every other hermes backend.
          </p>
          <ReachabilityProbe />
        </div>
      ) : (
        <p className="text-sm text-[var(--text-secondary)]">
          Set NEXT_PUBLIC_HERMES_GATEWAY_PUBLIC_URL to advertise this agent&apos;s
          gateway.
        </p>
      )}
    </ConnectionCard>
  );
}

function CopyField({
  label,
  value,
  sensitive = false,
}: {
  label: string;
  value: string;
  sensitive?: boolean;
}) {
  const [copied, setCopied] = useState(false);
  const [revealed, setRevealed] = useState(!sensitive);
  const copy = useCallback(() => {
    void navigator.clipboard.writeText(value).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [value]);
  return (
    <div className="flex items-center gap-2">
      <span className="w-24 shrink-0 text-xs font-medium text-[var(--text-secondary)]">
        {label}
      </span>
      <code className="min-w-0 flex-1 truncate rounded-md bg-[var(--surface-subtle)] px-2 py-1.5 font-mono text-xs">
        {revealed ? value : "•".repeat(Math.min(value.length, 24))}
      </code>
      {sensitive ? (
        <button
          type="button"
          className="ocean-icon-button"
          aria-label={revealed ? `Hide ${label}` : `Reveal ${label}`}
          onClick={() => setRevealed((current) => !current)}
        >
          {revealed ? "Hide" : "Show"}
        </button>
      ) : null}
      <button
        type="button"
        className="ocean-icon-button"
        aria-label={`Copy ${label}`}
        title={`Copy ${label}`}
        onClick={copy}
      >
        {copied ? <CheckIcon className="size-4" /> : <CopyIcon className="size-4" />}
      </button>
    </div>
  );
}

/**
 * Proves the advertised URL is dialable from this client context: open a ws,
 * wait for gateway.ready, run session.list. The same handshake hermes desktop
 * performs (non-browser clients connect directly; no proxy involved).
 */
function ReachabilityProbe() {
  const [state, setState] = useState<"idle" | "probing" | "ok" | "fail">("idle");
  const [detail, setDetail] = useState<string | null>(null);
  const disposedRef = useRef(false);
  useEffect(() => () => {
    disposedRef.current = true;
  }, []);

  const probe = useCallback(() => {
    setState("probing");
    setDetail(null);
    const target = TOKEN
      ? `${PUBLIC_URL}?token=${encodeURIComponent(TOKEN)}`
      : PUBLIC_URL;
    let socket: WebSocket;
    try {
      socket = new WebSocket(target);
    } catch {
      setState("fail");
      setDetail("invalid URL");
      return;
    }
    const finish = (ok: boolean, text: string) => {
      if (disposedRef.current) return;
      setState(ok ? "ok" : "fail");
      setDetail(text);
      try {
        socket.close();
      } catch {
        /* already closed */
      }
    };
    const timer = setTimeout(() => finish(false, "timed out"), 8_000);
    socket.addEventListener("open", () => {
      socket.send(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "session.list" }));
    });
    socket.addEventListener("message", (event: MessageEvent) => {
      let message: { id?: number; result?: { sessions?: unknown[] }; error?: { message?: string } };
      try {
        message = JSON.parse(String(event.data));
      } catch {
        return;
      }
      if (message.id !== 1) return;
      clearTimeout(timer);
      if (message.error) {
        finish(false, message.error.message ?? "gateway error");
        return;
      }
      finish(true, `${message.result?.sessions?.length ?? 0} sessions reachable`);
    });
    socket.addEventListener("error", () => {
      clearTimeout(timer);
      finish(false, "could not connect");
    });
  }, []);

  return (
    <div className="flex items-center gap-2 pt-1">
      <button type="button" className="ocean-pill-button" onClick={probe}>
        {state === "probing" ? (
          <LoaderCircleIcon className="size-4 finite-chat__spin" />
        ) : (
          <PlugZapIcon className="size-4" />
        )}
        <span>{state === "probing" ? "Probing…" : "Test connection"}</span>
      </button>
      {detail ? (
        <span
          className={`text-xs ${state === "ok" ? "text-emerald-500" : "text-red-400"}`}
        >
          {detail}
        </span>
      ) : null}
    </div>
  );
}
