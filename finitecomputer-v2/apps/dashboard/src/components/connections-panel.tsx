"use client";

import { FormEvent, useCallback, useEffect, useRef, useState } from "react";
import {
  BriefcaseBusinessIcon,
  ExternalLinkIcon,
  RefreshCwIcon,
  SendIcon,
  UnplugIcon,
} from "lucide-react";

import { SimplexConnection } from "@/components/simplex-connection";
import { ConnectionCard } from "@/components/connection-card";
import { useOptionalHostedChat } from "@/components/hosted-chat-provider";
import { InferenceConnections, type InferenceOutcome } from "@/components/inference-connections";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type {
  AgentConnectionAction,
  AgentConnectionActionResult,
  AgentConnectionsStatus,
} from "@/lib/hosted-agent-controls";

export const CONNECTIONS_REQUEST_TIMEOUT_MS = 20_000;
export const CONNECTIONS_MUTATION_TIMEOUT_MS = 60_000;
export const CONNECTIONS_TIMEOUT_MESSAGE =
  "Your agent is taking longer than expected. Try again.";

export function ConnectionsPanel({
  machineId,
  googleConfigured,
}: {
  machineId: string;
  googleConfigured: boolean;
}) {
  const [status, setStatus] = useState<AgentConnectionsStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const chat = useOptionalHostedChat();
  const refreshedChatDevicesRef = useRef(false);
  const endpoint = `/api/connections/machines/${encodeURIComponent(machineId)}`;

  const refresh = useCallback(async (quiet = false) => {
    if (!quiet) setError(null);
    try {
      setStatus(await connectionRequest(endpoint));
    } catch (requestError) {
      if (!quiet) setError(connectionErrorMessage(requestError));
    }
  }, [endpoint]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const refreshAfterExternalConnection = () => void refresh();
    window.addEventListener("focus", refreshAfterExternalConnection);
    return () => window.removeEventListener("focus", refreshAfterExternalConnection);
  }, [refresh]);

  useEffect(() => {
    if (!chat?.state || refreshedChatDevicesRef.current) return;
    refreshedChatDevicesRef.current = true;
    void chat.dispatchQuiet({ RefreshDevices: null });
  }, [chat]);

  useEffect(() => {
    if (!status?.simplex?.enabled || busy) return;
    let inFlight = false;
    const timer = window.setInterval(async () => {
      if (inFlight || document.hidden) return;
      inFlight = true;
      try { await refresh(true); } finally { inFlight = false; }
    }, 3000);
    return () => window.clearInterval(timer);
  }, [status?.simplex?.enabled, busy, refresh]);

  async function mutate(label: string, action: AgentConnectionAction) {
    setBusy(label);
    setError(null);
    try {
      setStatus(
        await connectionRequest(
          endpoint,
          {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify(action),
          },
          CONNECTIONS_MUTATION_TIMEOUT_MS
        )
      );
    } catch (requestError) {
      setError(connectionErrorMessage(requestError));
    } finally {
      setBusy(null);
    }
  }

  // Inference results are shown next to the action that produced them, with agentd's code.
  async function sendInference(action: AgentConnectionAction): Promise<InferenceOutcome> {
    setBusy("inference");
    setError(null);
    try {
      const payload = await connectionRequest<AgentConnectionsStatus | AgentConnectionActionResult>(
        endpoint,
        {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(action),
        },
        CONNECTIONS_MUTATION_TIMEOUT_MS
      );
      if ("result" in payload) {
        setStatus(payload.status);
        return { ok: true, result: payload.result };
      }
      setStatus(payload);
      return { ok: true, result: null };
    } catch (requestError) {
      return {
        ok: false,
        code: requestError instanceof ConnectionRequestError ? requestError.code : null,
        message: connectionErrorMessage(requestError),
      };
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="space-y-4">
      {status && error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
      {!status ? (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border bg-white/[0.03] px-4 py-3 text-sm">
          <span>
            {error
              ? `${error} The controls remain visible for local UI development.`
              : "Checking live connection status… Controls will unlock when the agent responds."}
          </span>
          <Button variant="outline" size="sm" onClick={() => void refresh()}>
            <RefreshCwIcon />
            Try again
          </Button>
        </div>
      ) : null}

      <InferenceConnections status={status} busy={Boolean(busy)} send={sendInference} refresh={refresh} />

      <SimplexConnection status={status?.simplex} loaded={Boolean(status)} busy={Boolean(busy)} mutate={mutate} refresh={refresh} />

      <ConnectionCard
        name="Telegram"
        state={!status ? "unavailable" : status.telegram.connected ? "connected" : "disconnected"}
        account={status?.telegram.home_channel ?? null}
        description="Talk to the same agent from Telegram."
        icon={<SendIcon className="size-5" />}
        footer={
          <TelegramDetails
            status={status}
            busy={busy}
            refresh={refresh}
            mutate={mutate}
          />
        }
      >
        <TelegramPrimary status={status} busy={busy} mutate={mutate} />
      </ConnectionCard>

      <ConnectionCard
        name="Google Workspace"
        state={!status ? "unavailable" : status.google.connected ? "connected" : "disconnected"}
        account={status?.google.email ?? null}
        description="Let your agent work with Gmail, Calendar, Drive, Docs, and Sheets."
        icon={<BriefcaseBusinessIcon className="size-5" />}
        error={!googleConfigured ? "Google sign-in is not configured yet." : null}
      >
        <div className="flex flex-wrap justify-end gap-2">
          {status?.google.connected ? (
            <Button
              variant="outline"
              disabled={Boolean(busy)}
              onClick={() => void mutate("google", { action: "google_disconnect" })}
            >
              <UnplugIcon />
              {busy === "google" ? "Disconnecting…" : "Disconnect"}
            </Button>
          ) : null}
          {googleConfigured && status ? (
            <Button asChild disabled={Boolean(busy)}>
              <a
                href={`/google-workspace/start?machineId=${encodeURIComponent(machineId)}`}
              >
                {status?.google.connected ? "Reconnect" : "Connect"}
                <ExternalLinkIcon />
              </a>
            </Button>
          ) : (
            <Button disabled>Connect</Button>
          )}
        </div>
      </ConnectionCard>

      {chat?.state ? (
        <section className="rounded-xl border border-border bg-white/[0.03] p-4">
          <div className="mb-3">
            <h2 className="font-medium">Chat devices · account-wide</h2>
            <p className="text-sm text-muted-foreground">
              Read-only devices attached to this chat account.
            </p>
          </div>
          <div className="overflow-x-auto">
            <table className="w-full min-w-[38rem] text-left text-sm">
              <thead className="text-xs text-muted-foreground">
                <tr className="border-b border-border/70">
                  <th className="px-2 py-2 font-medium">Device ID</th>
                  <th className="px-2 py-2 font-medium">Current</th>
                  <th className="px-2 py-2 font-medium">Active</th>
                  <th className="px-2 py-2 font-medium">Revoked</th>
                  <th className="px-2 py-2 text-right font-medium">Rooms</th>
                </tr>
              </thead>
              <tbody>
                {chat.state.devices.map((device) => (
                  <tr key={`${device.account_id}:${device.device_id}`} className="border-b border-border/40 last:border-0">
                    <td className="px-2 py-2 font-mono text-xs">{device.device_id}</td>
                    <td className="px-2 py-2">{yesNo(device.current_device)}</td>
                    <td className="px-2 py-2">{yesNo(device.active)}</td>
                    <td className="px-2 py-2">{yesNo(device.revoked)}</td>
                    <td className="px-2 py-2 text-right tabular-nums">{device.room_count}</td>
                  </tr>
                ))}
                {chat.state.devices.length === 0 ? (
                  <tr>
                    <td colSpan={5} className="px-2 py-4 text-muted-foreground">
                      No chat devices reported.
                    </td>
                  </tr>
                ) : null}
              </tbody>
            </table>
          </div>
        </section>
      ) : null}
    </div>
  );
}

function TelegramPrimary({
  status,
  busy,
  mutate,
}: {
  status: AgentConnectionsStatus | null;
  busy: string | null;
  mutate: (label: string, action: AgentConnectionAction) => Promise<void>;
}) {
  if (status?.telegram.connected) {
    return (
      <Button
        variant="outline"
        disabled={Boolean(busy)}
        onClick={() => void mutate("telegram", { action: "telegram_disconnect" })}
      >
        <UnplugIcon />
        {busy === "telegram" ? "Disconnecting…" : "Disconnect"}
      </Button>
    );
  }
  return (
    <form
      className="flex min-w-0 flex-wrap items-center justify-end gap-2"
      onSubmit={(event) => submitTelegramToken(event, mutate)}
    >
      <Input
        name="token"
        type="password"
        autoComplete="off"
        aria-label="Telegram bot token"
        placeholder="Bot token"
        className="w-56"
      />
      <Button type="submit" disabled={Boolean(busy) || !status}>
        {busy === "telegram" ? "Connecting…" : "Connect"}
      </Button>
    </form>
  );
}

function TelegramDetails({
  status,
  busy,
  refresh,
  mutate,
}: {
  status: AgentConnectionsStatus | null;
  busy: string | null;
  refresh: () => Promise<void>;
  mutate: (label: string, action: AgentConnectionAction) => Promise<void>;
}) {
  if (!status) return null;
  if (!status.telegram.connected) {
    return (
      <p className="text-sm text-muted-foreground">
        Open{" "}
        <a
          href="https://t.me/BotFather"
          target="_blank"
          rel="noreferrer"
          className="font-medium text-foreground underline underline-offset-4"
        >
          @BotFather
        </a>
        , send <code>/newbot</code>, choose a name and a username ending in <code>bot</code>,
        then paste the token above.
      </p>
    );
  }
  return (
    <div className="grid gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-sm text-muted-foreground">
          Message your bot. It will show an eight-character code; enter it here.
        </p>
        <Button variant="ghost" size="sm" onClick={() => void refresh()}>
          <RefreshCwIcon />
          Refresh
        </Button>
      </div>
      <form
        className="flex flex-wrap gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          const form = new FormData(event.currentTarget);
          void mutate("telegram", {
            action: "telegram_approve",
            code: String(form.get("code") ?? ""),
          });
        }}
      >
        <Input
          name="code"
          autoComplete="off"
          maxLength={8}
          placeholder="Pairing code"
          aria-label="Telegram pairing code"
          className="w-44 uppercase"
        />
        <Button type="submit" variant="outline" disabled={Boolean(busy)}>
          {busy === "telegram" ? "Approving…" : "Approve"}
        </Button>
      </form>
      {status.telegram.pending.length > 0 ? (
        <p className="text-xs text-muted-foreground">
          {status.telegram.pending.length} pairing request
          {status.telegram.pending.length === 1 ? " is" : "s are"} waiting.
        </p>
      ) : null}
      {status.telegram.approved.map((person) => (
        <div
          key={person.user_id}
          className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-border/70 px-3 py-2"
        >
          <span className="text-sm">{person.name || `Telegram ${person.user_id}`}</span>
          <Button
            size="sm"
            variant="outline"
            disabled={Boolean(busy)}
            onClick={() =>
              void mutate("telegram", {
                action: "telegram_home",
                userId: person.user_id,
                name: person.name,
              })
            }
          >
            Use this chat
          </Button>
        </div>
      ))}
    </div>
  );
}

function submitTelegramToken(
  event: FormEvent<HTMLFormElement>,
  mutate: (label: string, action: AgentConnectionAction) => Promise<void>
) {
  event.preventDefault();
  const form = new FormData(event.currentTarget);
  void mutate("telegram", {
    action: "telegram_connect",
    token: String(form.get("token") ?? ""),
  });
}

function yesNo(value: boolean) {
  return value ? "Yes" : "No";
}

/** A failed Connections request, with the agentd or dashboard error code when the server sent one. */
export class ConnectionRequestError extends Error {
  constructor(
    message: string,
    readonly code: string | null
  ) {
    super(message);
  }
}

export async function connectionRequest<T = AgentConnectionsStatus>(
  endpoint: string,
  init?: RequestInit,
  timeoutMs = CONNECTIONS_REQUEST_TIMEOUT_MS
): Promise<T> {
  const response = await fetch(endpoint, {
    ...init,
    cache: "no-store",
    signal: AbortSignal.timeout(timeoutMs),
  });
  const payload = (await response.json().catch(() => ({}))) as {
    error?: unknown;
    code?: unknown;
  };
  if (!response.ok) {
    throw new ConnectionRequestError(
      typeof payload.error === "string" && payload.error.trim()
        ? payload.error
        : "Connections are unavailable right now.",
      typeof payload.code === "string" ? payload.code : null
    );
  }
  return payload as T;
}

export function connectionErrorMessage(error: unknown) {
  if (
    error instanceof Error
    && (error.name === "TimeoutError" || error.name === "AbortError")
  ) {
    return CONNECTIONS_TIMEOUT_MESSAGE;
  }
  return error instanceof Error ? error.message : "Connections are unavailable right now.";
}
