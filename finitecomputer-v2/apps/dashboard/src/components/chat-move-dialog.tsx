"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { sidebarChatKey, type SidebarChat, type SidebarTopic } from "@/lib/hosted-web-chat-topics";

export function ChatMoveDialog({ chat, topics, busy, onClose, onMove }: {
  chat: SidebarChat | null;
  topics: SidebarTopic[];
  busy: boolean;
  onClose: () => void;
  onMove: (chat: SidebarChat, topicId: string, before: SidebarChat | null) => Promise<boolean>;
}) {
  return <Dialog open={Boolean(chat)} onOpenChange={(open) => { if (!open && !busy) onClose(); }}>
    <DialogContent>
      {chat ? <MoveForm key={sidebarChatKey(chat)} chat={chat} topics={topics}
        busy={busy} onClose={onClose} onMove={onMove} /> : null}
    </DialogContent>
  </Dialog>;
}

function MoveForm({ chat, topics, busy, onClose, onMove }: {
  chat: SidebarChat;
  topics: SidebarTopic[];
  busy: boolean;
  onClose: () => void;
  onMove: (chat: SidebarChat, topicId: string, before: SidebarChat | null) => Promise<boolean>;
}) {
  const [topicId, setTopicId] = useState(chat.placement?.topic_id ?? chat.source_topic_id);
  const [beforeKey, setBeforeKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const candidates = (topics.find((topic) => topic.topic_id === topicId)?.chats ?? [])
    .filter((item) => !item.archived && sidebarChatKey(item) !== sidebarChatKey(chat));
  const valid = topics.some((topic) => topic.topic_id === topicId)
    && (!beforeKey || candidates.some((item) => sidebarChatKey(item) === beforeKey));
  return <form className="finite-chat__rename-form" onSubmit={async (event) => {
    event.preventDefault();
    if (!valid || busy) return;
    setError(null);
    if (await onMove(chat, topicId, candidates.find((item) => sidebarChatKey(item) === beforeKey) ?? null)) onClose();
    else setError("Could not move the chat. Check the topic and position, then try again.");
  }}>
    <DialogHeader>
      <DialogTitle>Move chat</DialogTitle>
      <DialogDescription>Choose where to put {chat.title || "this chat"}.</DialogDescription>
    </DialogHeader>
    <div className="finite-chat__rename-field">
      <label htmlFor="move-chat-topic">Topic</label>
      <select id="move-chat-topic" className="finite-chat__move-select" value={topicId} disabled={busy}
        onChange={(event) => { setTopicId(event.target.value); setBeforeKey(""); }}>
        {topics.map((topic) => <option key={topic.topic_id} value={topic.topic_id}>{topic.title}</option>)}
      </select>
    </div>
    <div className="finite-chat__rename-field">
      <label htmlFor="move-chat-position">Position</label>
      <select id="move-chat-position" className="finite-chat__move-select" value={beforeKey} disabled={busy}
        onChange={(event) => setBeforeKey(event.target.value)}>
        {candidates.map((item) => <option key={sidebarChatKey(item)} value={sidebarChatKey(item)}>Before {item.title || "New chat"}</option>)}
        <option value="">At the end</option>
      </select>
    </div>
    {error ? <p role="alert">{error}</p> : null}
    {!valid ? <p role="status">The chats changed. Choose a position again.</p> : null}
    <DialogFooter>
      <Button type="button" variant="outline" disabled={busy} onClick={onClose}>Cancel</Button>
      <Button type="submit" disabled={busy || !valid}>{busy ? "Moving…" : "Move chat"}</Button>
    </DialogFooter>
  </form>;
}
