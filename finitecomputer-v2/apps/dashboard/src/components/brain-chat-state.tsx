"use client";

import { useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { BrainIcon } from "lucide-react";
import { useHostedChat } from "@/components/hosted-chat-provider";
import { canonicalNewChatTopic } from "@/lib/hosted-web-chat-topics";

export function BrainChatState({ agentName, machineId, unavailable = false }: { agentName: string; machineId: string; unavailable?: boolean }) {
  const { state, dispatch } = useHostedChat();
  const router = useRouter();
  const pending = useRef(false);
  const intentKey = useRef<string | null>(null);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function openChat() {
    if (pending.current) return;
    const roomId = state?.hosted_agent_binding?.canonical_room_id;
    const topic = canonicalNewChatTopic((state?.topics ?? []).filter((item) => item.room_id === roomId && !item.archived));
    if (!roomId || !topic) {
      setError("Chat is still connecting. Please try again in a moment.");
      return;
    }
    pending.current = true;
    setOpening(true);
    setError(null);
    intentKey.current ??= crypto.randomUUID();
    try {
      await dispatch({ StartTopicChatIntent: { room_id: roomId, topic_id: topic.topic_id, reason: null, intent_key: intentKey.current } });
      const prompt = unavailable
        ? "Help me with Brain. Show me which brains you can access, or help me create one."
        : "Help me create a brain. Start by asking whether I want a personal brain or an organization brain.";
      router.push(`/dashboard/machines/${encodeURIComponent(machineId)}/chat?${new URLSearchParams({ prompt })}`);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Couldn’t open a new chat. Please try again.");
      pending.current = false;
      setOpening(false);
    }
  }

  return (
    <div className="brain-empty-state">
      <BrainIcon aria-hidden="true" />
      <h2>{unavailable ? "Brain overview isn’t available yet" : "Your brains will show up here"}</h2>
      <p>{unavailable
        ? `You can still ask ${agentName} about your brains or create one in chat.`
        : `Talk to ${agentName} to create your personal brain or an organization brain for your team.`}</p>
      <button type="button" className="brain-empty-chat" disabled={opening} onClick={() => void openChat()}>{opening ? "Opening chat…" : "Open chat"}</button>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
