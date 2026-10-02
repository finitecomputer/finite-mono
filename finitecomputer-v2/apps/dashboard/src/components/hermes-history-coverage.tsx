"use client";

import { useEffect, useState } from "react";

import { parseHermesSessionPage, type HermesSessionRow } from "@/lib/hermes-chat-sessions";
import { hermesHistoryCoverage, type HermesCoverage } from "@/lib/hermes-history-coverage";
import { readHostedHermesJson } from "@/lib/hosted-hermes-status";
import type { HostedChatState } from "@/lib/hosted-web-device";

const SESSION_PAGE_SIZE = 100;
const MAX_SESSION_PAGES = 20;

/** Admin spot-check: Finite Chat chats this agent's Hermes can and cannot show. */
export function HermesHistoryCoverage({ runtimeId }: { runtimeId: string }) {
  const [coverage, setCoverage] = useState<HermesCoverage | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    void (async () => {
      try {
        const [finiteChat, sessions] = await Promise.all([
          readFiniteChatState(runtimeId, controller.signal),
          readHermesSessions(runtimeId, controller.signal),
        ]);
        const result = hermesHistoryCoverage(finiteChat, sessions);
        if (!result) throw new Error("This agent has no Finite Chat room for your account.");
        setCoverage(result);
      } catch (caught) {
        if (!controller.signal.aborted) {
          setError(caught instanceof Error ? caught.message : "Coverage is unavailable.");
        }
      }
    })();
    return () => controller.abort();
  }, [runtimeId]);

  if (error) return <p role="alert">{error}</p>;
  if (!coverage) return <p>Comparing Finite Chat with Hermes…</p>;
  return (
    <div className="space-y-4">
      <p>
        Hermes has {coverage.covered} of {coverage.chatsWithMessages} Finite Chat chats with messages.
        {coverage.missing.length ? ` ${coverage.missing.length} are missing.` : " None are missing."}
      </p>
      <p className="text-sm text-muted-foreground">
        Message counts differ by design: Finite Chat also counts tool progress, status notices
        and edits. A chat listed as missing is history only Finite Chat holds.
      </p>
      <table className="w-full text-sm">
        <thead>
          <tr className="text-left">
            <th>Topic</th>
            <th>Chat</th>
            <th className="text-right">Finite Chat</th>
            <th className="text-right">Hermes</th>
          </tr>
        </thead>
        <tbody>
          {[...coverage.missing, ...coverage.chats.filter((chat) => chat.hermesSessions > 0)].map((chat) => (
            <tr key={chat.chatId} data-missing={chat.hermesSessions === 0 || undefined}>
              <td>{chat.topicTitle}</td>
              <td>{chat.title}</td>
              <td className="text-right">{chat.finiteChatMessages}</td>
              <td className="text-right">
                {chat.hermesMessages === null ? "missing" : chat.hermesMessages}
                {chat.hermesSessions > 1 ? ` (${chat.hermesSessions} sessions)` : ""}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

async function readFiniteChatState(runtimeId: string, signal: AbortSignal): Promise<HostedChatState> {
  const response = await fetch(
    `/api/chat/machines/${encodeURIComponent(runtimeId)}/hosted-device/state`,
    { cache: "no-store", credentials: "same-origin", signal },
  );
  if (!response.ok) throw new Error("Finite Chat history is unavailable for this agent.");
  return await response.json() as HostedChatState;
}

async function readHermesSessions(runtimeId: string, signal: AbortSignal): Promise<HermesSessionRow[]> {
  const sessions: HermesSessionRow[] = [];
  for (let page = 0; page < MAX_SESSION_PAGES; page++) {
    const result = parseHermesSessionPage(await readHostedHermesJson(runtimeId, "api/sessions", signal, {
      query: { archived: "include", order: "recent", limit: SESSION_PAGE_SIZE, offset: page * SESSION_PAGE_SIZE },
      maxBytes: 4 * 1024 * 1024,
    }));
    sessions.push(...result.sessions);
    if (result.sessions.length < SESSION_PAGE_SIZE || sessions.length >= result.total) break;
  }
  return sessions;
}
