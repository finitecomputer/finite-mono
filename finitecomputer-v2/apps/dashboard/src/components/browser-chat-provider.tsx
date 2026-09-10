"use client";
import { useEffect, useState, type ReactNode } from "react";
import { HostedChatContext, type HostedChatContextValue } from "./hosted-chat-provider";
import { browserFiniteChatSession, type BrowserFiniteChat } from "@/lib/browser-finitechat";

export function BrowserChatProvider({ children }: { children: ReactNode; machineId: string }) {
  const [client, setClient] = useState<BrowserFiniteChat | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [, update] = useState(0);
  useEffect(() => {
    let active = true;
    let unsubscribe: (() => void) | undefined;
    let timer: ReturnType<typeof setTimeout>;
    async function start() {
      try {
        const session = await browserFiniteChatSession();
        if (!active) return;
        setClient(session);
        unsubscribe = session.subscribe(() => update(value => value + 1));
        const poll = async () => {
          try { await session.sync(); } catch { /* Exposed as transportError; retry on next poll. */ }
          if (active) timer = setTimeout(poll, 500);
        };
        timer = setTimeout(poll, 500);
      } catch (error) { if (active) setError(String(error)); }
    }
    void start();
    return () => { active = false; clearTimeout(timer); unsubscribe?.(); };
  }, []);
  const ready = async () => client ?? await browserFiniteChatSession();
  const load: HostedChatContextValue["load"] = async () => {
    try { await (await ready()).sync(); return "succeeded"; } catch { return "retry"; }
  };
  const value: HostedChatContextValue = {
    capabilities: { attachments: false, brain: false },
    apiBase: "", state: client?.state ?? null, transportError: error ?? client?.error ?? null,
    claimError: null, streamConnected: client?.connected ?? false, ownerClaimed: client !== null,
    bindingRecoveryRequired: false, selectionPending: false, load, claimOwner: load, recoverBinding: load,
    dispatch: async action => (await ready()).dispatch(action),
    dispatchQuiet: async action => { try { return await (await ready()).dispatch(action); } catch { return null; } },
    refreshPendingChat: async () => { await (await ready()).sync(); return true; },
    uploadAttachments: async () => { throw new Error("Attachments are not implemented in the browser spike yet."); },
    attachmentUrl: () => { throw new Error("Attachment downloads are not implemented in the browser spike yet."); },
  };
  return <HostedChatContext.Provider value={value}>{children}</HostedChatContext.Provider>;
}
