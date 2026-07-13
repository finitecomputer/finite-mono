"use client";

import type { CSSProperties, ReactNode } from "react";
import { useCallback, useMemo, useState } from "react";
import { usePathname, useRouter } from "next/navigation";
import {
  HashIcon,
  PanelLeftIcon,
  PlusIcon,
  RotateCcwIcon,
} from "lucide-react";

import { AccountMenu, AgentNavigation } from "@/components/agent-navigation";
import { FiniteBrand } from "@/components/finite-brand";
import { useHostedChat } from "@/components/hosted-chat-provider";
import { Button } from "@/components/ui/button";
import type {
  HostedChatAction,
  HostedChatSummary,
  HostedChatTopic,
} from "@/lib/hosted-web-device";
import { canonicalNewChatTopic, HOME_TOPIC_ID } from "@/lib/hosted-web-chat-topics";

export function AgentSidebar({
  collapsed,
  machineId,
  machineLabel,
  machineSwitcher,
  mobileOpen,
  onCollapsedChange,
  onMobileOpenChange,
  showSkills,
  viewerEmail,
}: {
  collapsed: boolean;
  machineId: string;
  machineLabel: string;
  machineSwitcher: ReactNode;
  mobileOpen: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
  onMobileOpenChange: (open: boolean) => void;
  showSkills: boolean;
  viewerEmail?: string | null;
}) {
  const pathname = usePathname() ?? "";
  const router = useRouter();
  const {
    state,
    transportError,
    bindingRecoveryRequired,
    load,
    recoverBinding,
    dispatch,
  } = useHostedChat();
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const canonicalRoomId = state?.hosted_agent_binding?.canonical_room_id ?? null;
  const topics = useMemo(
    () => (state?.topics ?? [])
      .filter((topic) => topic.room_id === canonicalRoomId && !topic.archived)
      .sort((left, right) => {
        if (left.topic_id === HOME_TOPIC_ID) return -1;
        if (right.topic_id === HOME_TOPIC_ID) return 1;
        return right.updated_seq - left.updated_seq || left.title.localeCompare(right.title);
      }),
    [canonicalRoomId, state?.topics]
  );
  const previousRoomIds = useMemo(
    () => new Set(
      (state?.hosted_agent_binding?.associated_room_ids ?? [])
        .filter((roomId) => roomId !== canonicalRoomId)
    ),
    [canonicalRoomId, state?.hosted_agent_binding?.associated_room_ids]
  );
  const previousTopics = useMemo(
    () => (state?.topics ?? [])
      .filter((topic) => previousRoomIds.has(topic.room_id) && !topic.archived)
      .sort((left, right) => right.updated_seq - left.updated_seq || left.title.localeCompare(right.title)),
    [previousRoomIds, state?.topics]
  );
  const selectedTopicId = state?.selected_topic_id ?? null;
  const selectedChatId = state?.selected_chat_id ?? null;
  const defaultNewChatTopic = canonicalNewChatTopic(topics);

  const act = useCallback(async (action: HostedChatAction) => {
    setBusy(true);
    try {
      const next = await dispatch(action);
      setActionError(null);
      if (!pathname.endsWith("/chat")) {
        router.push(`/dashboard/machines/${encodeURIComponent(machineId)}/chat`);
      }
      onMobileOpenChange(false);
      return next;
    } catch (caught) {
      setActionError(caught instanceof Error
        ? caught.message
        : "That chat action is temporarily unavailable.");
      return null;
    } finally {
      setBusy(false);
    }
  }, [dispatch, machineId, onMobileOpenChange, pathname, router]);

  function openTopic(topic: HostedChatTopic) {
    void act({ OpenTopic: { room_id: topic.room_id, topic_id: topic.topic_id } });
  }

  function openChat(topic: HostedChatTopic, chat: HostedChatSummary) {
    void act({
      OpenChat: {
        room_id: topic.room_id,
        topic_id: topic.topic_id,
        chat_id: chat.chat_id,
      },
    });
  }

  function createChat(topic: HostedChatTopic | null) {
    if (!canonicalRoomId || !topic) return;
    void act({
      StartTopicChatIntent: {
        room_id: canonicalRoomId,
        topic_id: topic.topic_id,
        reason: null,
        intent_key: crypto.randomUUID(),
      },
    });
  }

  return (
    <>
      {mobileOpen ? (
        <button
          type="button"
          className="finite-chat__sidebar-backdrop"
          aria-label="Close agent navigation"
          onClick={() => onMobileOpenChange(false)}
        />
      ) : null}
      <aside className={`finite-chat__sidebar finite-agent-shell__sidebar ${mobileOpen ? "is-open" : ""}`}>
        <div className="finite-chat__sidebar-top">
          <div className="finite-chat__brand"><FiniteBrand href="/dashboard" /></div>
          <button
            type="button"
            className="ocean-icon-button finite-chat__desktop-collapse-button"
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
            aria-pressed={collapsed}
            onClick={() => onCollapsedChange(!collapsed)}
          >
            <PanelLeftIcon className="size-4" />
          </button>
          <button
            type="button"
            className="ocean-icon-button finite-chat__mobile-collapse-button"
            aria-label="Close agent navigation"
            onClick={() => onMobileOpenChange(false)}
          >
            <PanelLeftIcon className="size-4" />
          </button>
        </div>

        <div className="finite-agent-shell__machine">{machineSwitcher}</div>

        <nav className="finite-chat__sidebar-nav" aria-label="Agent, topics, and chats">
          <AgentNavigation
            machineId={machineId}
            onNavigate={() => onMobileOpenChange(false)}
            showSkills={showSkills}
          />
          <div className="finite-chat__sidebar-section-row">
            <span className="finite-chat__sidebar-section">Topics</span>
          </div>
          {!state && !transportError ? <p className="finite-agent-sidebar__status">Loading chats…</p> : null}
          {transportError ? (
            <div className="finite-agent-sidebar__error">
              <span>{transportError}</span>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => void (bindingRecoveryRequired ? recoverBinding() : load())}
              >
                <RotateCcwIcon />
                {bindingRecoveryRequired ? "Finish chat setup" : "Retry"}
              </Button>
            </div>
          ) : null}
          {actionError ? (
            <div className="finite-agent-sidebar__error">
              <span>{actionError}</span>
              <Button type="button" variant="ghost" size="sm" onClick={() => setActionError(null)}>
                Dismiss
              </Button>
            </div>
          ) : null}
          {topics.map((topic) => (
            <div className="finite-chat__folder" key={`${topic.room_id}:${topic.topic_id}`}>
              <div className="finite-chat__folder-header">
                <button type="button" className="finite-chat__folder-summary" onClick={() => openTopic(topic)}>
                  <span className="finite-chat__folder-main">
                    <span className="finite-chat__folder-icon" style={topicColorStyle(topic.title)} aria-hidden>
                      <HashIcon className="size-3.5" />
                    </span>
                    <span className="finite-chat__folder-label">{topic.title}</span>
                  </span>
                  {topic.unread_count > 0 ? <span className="finite-chat__unread-count">{topic.unread_count}</span> : null}
                </button>
                <button
                  type="button"
                  className="finite-chat__topic-new-chat"
                  aria-label={`New chat in ${topic.title}`}
                  disabled={busy}
                  onClick={() => createChat(topic)}
                >
                  <PlusIcon className="size-3.5" />
                </button>
              </div>
              <div className="finite-chat__folder-body">
                {topic.chats.map((chat) => {
                  const active = topic.topic_id === selectedTopicId && chat.chat_id === selectedChatId;
                  return (
                    <button
                      key={chat.chat_id}
                      type="button"
                      className={active ? "is-active" : ""}
                      aria-current={active ? "page" : undefined}
                      onClick={() => openChat(topic, chat)}
                    >
                      <span className="finite-chat__thread-indicator" aria-hidden />
                      <span className="finite-chat__thread-main">
                        <span className="finite-chat__thread-title">{chat.title || "New chat"}</span>
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          ))}
          {previousTopics.length > 0 ? (
            <>
              <div className="finite-chat__sidebar-section-row">
                <span className="finite-chat__sidebar-section">Previous conversations</span>
              </div>
              {previousTopics.map((topic) => (
                <div className="finite-chat__folder" key={`previous:${topic.room_id}:${topic.topic_id}`}>
                  <div className="finite-chat__folder-header">
                    <button type="button" className="finite-chat__folder-summary" onClick={() => openTopic(topic)}>
                      <span className="finite-chat__folder-main">
                        <span className="finite-chat__folder-icon" style={topicColorStyle(topic.title)} aria-hidden>
                          <HashIcon className="size-3.5" />
                        </span>
                        <span className="finite-chat__folder-label">{topic.title}</span>
                      </span>
                    </button>
                  </div>
                  <div className="finite-chat__folder-body">
                    {topic.chats.map((chat) => {
                      const active = topic.room_id === state?.selected_room_id
                        && topic.topic_id === selectedTopicId
                        && chat.chat_id === selectedChatId;
                      return (
                        <button
                          key={chat.chat_id}
                          type="button"
                          className={active ? "is-active" : ""}
                          aria-current={active ? "page" : undefined}
                          onClick={() => openChat(topic, chat)}
                        >
                          <span className="finite-chat__thread-indicator" aria-hidden />
                          <span className="finite-chat__thread-main">
                            <span className="finite-chat__thread-title">{chat.title || "New chat"}</span>
                          </span>
                        </button>
                      );
                    })}
                  </div>
                </div>
              ))}
            </>
          ) : null}
        </nav>

        <button
          type="button"
          className="finite-chat__sidebar-new-chat-fab"
          disabled={busy || !defaultNewChatTopic}
          onClick={() => createChat(defaultNewChatTopic)}
        >
          <PlusIcon className="size-4" />
          <span>New chat</span>
        </button>

        <div className="finite-chat__sidebar-footer">
          <AccountMenu fallbackLabel={machineLabel} viewerEmail={viewerEmail} side="top" />
        </div>
      </aside>
    </>
  );
}

const TOPIC_COLORS = [
  ["#166534", "#dcfce7"],
  ["#1d4ed8", "#dbeafe"],
  ["#7e22ce", "#f3e8ff"],
  ["#9a3412", "#ffedd5"],
  ["#9f1239", "#ffe4e6"],
  ["#0f766e", "#ccfbf1"],
] as const;

function topicColorStyle(title: string): CSSProperties {
  let hash = 0;
  for (const character of title) hash = (hash * 31 + character.codePointAt(0)!) >>> 0;
  const [color, background] = TOPIC_COLORS[hash % TOPIC_COLORS.length]!;
  return { color, background };
}
