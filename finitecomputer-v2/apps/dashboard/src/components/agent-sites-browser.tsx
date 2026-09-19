"use client";

import { useRef, useState } from "react";
import { useRouter } from "next/navigation";

import { useHostedChat } from "@/components/hosted-chat-provider";
import { SitesBrowser, type SiteListItem } from "@/components/sites-browser";
import { canonicalNewChatTopic } from "@/lib/hosted-web-chat-topics";

export function AgentSitesBrowser({ machineId, sites, listingNotConnected = false }: { machineId: string; sites: readonly SiteListItem[] | null; listingNotConnected?: boolean }) {
  const { state, dispatch } = useHostedChat();
  const router = useRouter();
  const pending = useRef(false);
  const intent = useRef<{ target: string; key: string } | null>(null);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function openSiteChat(site?: SiteListItem) {
    if (pending.current) return;
    const roomId = state?.hosted_agent_binding?.canonical_room_id;
    const topic = canonicalNewChatTopic((state?.topics ?? []).filter((item) => item.room_id === roomId && !item.archived));
    if (!roomId || !topic) {
      setError("Chat is still connecting. Please try again in a moment.");
      return;
    }
    pending.current = true;
    setCreating(true);
    setError(null);
    // Reuse the sidebar's existing intent. Retry the same intent after an
    // uncertain response, and never send a message without the user's review.
    const target = site ? `edit:${site.id}` : "new";
    if (intent.current?.target !== target) intent.current = { target, key: crypto.randomUUID() };
    try {
      await dispatch({ StartTopicChatIntent: { room_id: roomId, topic_id: topic.topic_id, reason: null, intent_key: intent.current.key } });
      const prompt = site
        ? `Help me edit this site: ${site.title}\n${site.url}\n\nAsk me what I’d like to change.`
        : "Help me build a new website. Start by asking me what kind of site I want to create.";
      router.push(`/dashboard/machines/${encodeURIComponent(machineId)}/chat?${new URLSearchParams({ prompt })}`);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Couldn’t open a new chat. Please try again.");
      pending.current = false;
      setCreating(false);
    }
  }

  return <SitesBrowser sites={sites} listingNotConnected={listingNotConnected} onNewSite={() => void openSiteChat()} onEditSite={(site) => void openSiteChat(site)} creatingSite={creating} newSiteError={error} />;
}
