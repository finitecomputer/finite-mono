"use client";

import type { CSSProperties, FormEvent, ReactNode } from "react";
import { useCallback, useMemo, useState, useSyncExternalStore } from "react";
import { usePathname, useRouter } from "next/navigation";
import {
  ArchiveIcon,
  ArchiveRestoreIcon,
  ChevronRightIcon,
  HashIcon,
  MessageSquarePlusIcon,
  PanelLeftIcon,
  PencilIcon,
  PlusIcon,
  RotateCcwIcon,
} from "lucide-react";

import { AccountMenu, AgentNavigation } from "@/components/agent-navigation";
import { FiniteBrand } from "@/components/finite-brand";
import { useHostedChat } from "@/components/hosted-chat-provider";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { CHAT_TOPIC_DESCRIPTION } from "@/lib/chat-product-copy";
import {
  electronChatRuntime,
  electronRuntimeSupportsChatArchive,
} from "@/lib/electron-chat-runtime";
import type {
  HostedChatAction,
  HostedChatSummary,
  HostedChatTopic,
} from "@/lib/hosted-web-device";
import { canonicalNewChatTopic, HOME_TOPIC_ID } from "@/lib/hosted-web-chat-topics";

const subscribeHydration = () => () => undefined;

export function AgentSidebar({
  collapsed,
  machineId,
  machineLabel,
  machineSwitcher,
  mobileOpen,
  onCollapsedChange,
  onMobileOpenChange,
  viewerEmail,
}: {
  collapsed: boolean;
  machineId: string;
  machineLabel: string;
  machineSwitcher: ReactNode;
  mobileOpen: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
  onMobileOpenChange: (open: boolean) => void;
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
  const hydrated = useSyncExternalStore(
    subscribeHydration,
    () => true,
    () => false
  );
  const supportsChatArchive =
    hydrated && electronRuntimeSupportsChatArchive(electronChatRuntime());
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [createTopicOpen, setCreateTopicOpen] = useState(false);
  const [createTopicTitle, setCreateTopicTitle] = useState("");
  const [collapsedTopicKeys, setCollapsedTopicKeys] = useState<Set<string>>(
    () => new Set()
  );
  const [expandedArchiveKeys, setExpandedArchiveKeys] = useState<Set<string>>(
    () => new Set()
  );
  const [renameTarget, setRenameTarget] = useState<{
    topic: HostedChatTopic;
    chat: HostedChatSummary;
  } | null>(null);
  const [renameTitle, setRenameTitle] = useState("");

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
  const selectedTopicId = state?.selected_topic_id ?? null;
  const selectedChatId = state?.selected_chat_id ?? null;
  const defaultNewChatTopic = canonicalNewChatTopic(topics);

  const act = useCallback(async (action: HostedChatAction) => {
    setBusy(true);
    try {
      const canNavigateImmediately = "OpenTopic" in action || "OpenChat" in action;
      const navigatesAfterSuccess =
        "CreateTopic" in action || "StartTopicChatIntent" in action;
      const preservesMobileSidebar = "CreateTopic" in action;
      const pending = dispatch(action);
      if (canNavigateImmediately && !pathname.endsWith("/chat")) {
        router.push(`/dashboard/machines/${encodeURIComponent(machineId)}/chat`);
      }
      if (canNavigateImmediately) onMobileOpenChange(false);
      const next = await pending;
      setActionError(null);
      if (navigatesAfterSuccess) {
        if (!pathname.endsWith("/chat")) {
          router.push(`/dashboard/machines/${encodeURIComponent(machineId)}/chat`);
        }
        if (!preservesMobileSidebar) onMobileOpenChange(false);
      }
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

  function openChat(topic: HostedChatTopic, chat: HostedChatSummary) {
    void act({
      OpenChat: {
        room_id: topic.room_id,
        topic_id: topic.topic_id,
        chat_id: chat.chat_id,
      },
    });
  }

  function toggleTopicCollapsed(topicKey: string) {
    setCollapsedTopicKeys((current) => {
      const next = new Set(current);
      if (next.has(topicKey)) {
        next.delete(topicKey);
      } else {
        next.add(topicKey);
      }
      return next;
    });
  }

  function toggleArchiveExpanded(topicKey: string) {
    setExpandedArchiveKeys((current) => {
      const next = new Set(current);
      if (next.has(topicKey)) {
        next.delete(topicKey);
      } else {
        next.add(topicKey);
      }
      return next;
    });
  }

  function setChatArchived(
    topic: HostedChatTopic,
    chat: HostedChatSummary,
    archived: boolean
  ) {
    if (archived) {
      const topicKey = `${topic.room_id}:${topic.topic_id}`;
      setExpandedArchiveKeys((current) => new Set(current).add(topicKey));
    }
    void act({
      SetChatArchived: {
        room_id: topic.room_id,
        topic_id: topic.topic_id,
        chat_id: chat.chat_id,
        archived,
      },
    });
  }

  function openRename(topic: HostedChatTopic, chat: HostedChatSummary) {
    setRenameTarget({ topic, chat });
    setRenameTitle(chat.title || "New chat");
  }

  async function renameChat(event: FormEvent) {
    event.preventDefault();
    const title = renameTitle.trim();
    if (!renameTarget || !title || busy) return;
    const next = await act({
      RenameChat: {
        room_id: renameTarget.topic.room_id,
        topic_id: renameTarget.topic.topic_id,
        chat_id: renameTarget.chat.chat_id,
        title,
      },
    });
    if (next) setRenameTarget(null);
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

  async function createTopic(event: FormEvent) {
    event.preventDefault();
    const title = createTopicTitle.trim();
    if (!canonicalRoomId || !title || busy) return;
    const next = await act({
      CreateTopic: { room_id: canonicalRoomId, title },
    });
    if (!next) return;
    setCreateTopicTitle("");
    setCreateTopicOpen(false);
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
            <PanelLeftIcon className="size-3.5" />
          </button>
          <button
            type="button"
            className="ocean-icon-button finite-chat__mobile-collapse-button"
            aria-label="Close agent navigation"
            onClick={() => onMobileOpenChange(false)}
          >
            <PanelLeftIcon className="size-3.5" />
          </button>
        </div>

        <div className="finite-agent-shell__machine">{machineSwitcher}</div>

        <nav className="finite-chat__sidebar-nav" aria-label="Agent, topics, and chats">
          <AgentNavigation
            machineId={machineId}
            onNavigate={() => onMobileOpenChange(false)}
          />
          <div className="finite-chat__sidebar-section-row">
            <span className="finite-chat__sidebar-section">Topics</span>
            <button
              type="button"
              className="ocean-icon-button"
              aria-label="New topic"
              title="New topic"
              disabled={busy || !canonicalRoomId}
              onClick={() => setCreateTopicOpen(true)}
            >
              <PlusIcon className="size-3.5" />
            </button>
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
          {topics.map((topic) => {
            const topicKey = `${topic.room_id}:${topic.topic_id}`;
            const topicBodyId = `finite-chat-topic-${safeDomId(topicKey)}`;
            const archiveBodyId = `${topicBodyId}-archive`;
            const topicCollapsed = collapsedTopicKeys.has(topicKey);
            const visibleChats = supportsChatArchive
              ? topic.chats.filter((chat) => !chat.archived)
              : topic.chats;
            const archivedChats = supportsChatArchive
              ? topic.chats.filter((chat) => chat.archived)
              : [];
            const selectedChatIsArchived = archivedChats.some(
              (chat) => topic.topic_id === selectedTopicId && chat.chat_id === selectedChatId
            );
            const archiveExpanded =
              expandedArchiveKeys.has(topicKey) || selectedChatIsArchived;
            return (
              <div className="finite-chat__folder" key={topicKey}>
                <div className="finite-chat__folder-header">
                  <button
                    type="button"
                    className="finite-chat__folder-summary"
                    aria-controls={topicBodyId}
                    aria-expanded={!topicCollapsed}
                    aria-label={`${topicCollapsed ? "Expand" : "Collapse"} ${topic.title}`}
                    title={`${topicCollapsed ? "Expand" : "Collapse"} ${topic.title}`}
                    onClick={() => toggleTopicCollapsed(topicKey)}
                  >
                    <span className="finite-chat__folder-main">
                      <span className="finite-chat__folder-icon" style={topicColorStyle(topic.title)} aria-hidden>
                        <HashIcon className="size-3.5" />
                      </span>
                      <span className="finite-chat__folder-label">{topic.title}</span>
                    </span>
                    {topic.unread_count > 0 ? <span className="finite-chat__unread-count">{topic.unread_count}</span> : null}
                    <ChevronRightIcon className="finite-chat__topic-collapse-icon size-3.5" aria-hidden />
                  </button>
                  <button
                    type="button"
                    className="finite-chat__topic-new-chat"
                    aria-label={`New chat in ${topic.title}`}
                    title={`New chat in ${topic.title}`}
                    disabled={busy}
                    onClick={() => createChat(topic)}
                  >
                    <MessageSquarePlusIcon className="size-3.5" aria-hidden />
                  </button>
                </div>
                <div
                  id={topicBodyId}
                  className="finite-chat__folder-body"
                  hidden={topicCollapsed}
                >
                  {visibleChats.map((chat) => (
                    <ChatRow
                      key={chat.chat_id}
                      active={topic.topic_id === selectedTopicId && chat.chat_id === selectedChatId}
                      archived={false}
                      chat={chat}
                      disabled={busy}
                      onArchiveChange={supportsChatArchive
                        ? (archived) => setChatArchived(topic, chat, archived)
                        : undefined}
                      onOpen={() => openChat(topic, chat)}
                      onRename={() => openRename(topic, chat)}
                    />
                  ))}
                  {archivedChats.length > 0 ? (
                    <div className="finite-chat__archive-group">
                      <button
                        type="button"
                        className="finite-chat__archive-toggle"
                        aria-controls={archiveBodyId}
                        aria-expanded={archiveExpanded}
                        onClick={() => toggleArchiveExpanded(topicKey)}
                      >
                        <span>Archive</span>
                      </button>
                      <div id={archiveBodyId} hidden={!archiveExpanded}>
                        {archivedChats.map((chat) => (
                          <ChatRow
                            key={chat.chat_id}
                            active={topic.topic_id === selectedTopicId && chat.chat_id === selectedChatId}
                            archived
                            chat={chat}
                            disabled={busy}
                            onArchiveChange={(archived) => setChatArchived(topic, chat, archived)}
                            onOpen={() => openChat(topic, chat)}
                            onRename={() => openRename(topic, chat)}
                          />
                        ))}
                      </div>
                    </div>
                  ) : null}
                </div>
              </div>
            );
          })}
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

      <Dialog open={createTopicOpen} onOpenChange={setCreateTopicOpen}>
        <DialogContent>
          <form className="finite-chat__rename-form" onSubmit={createTopic}>
            <DialogHeader>
              <DialogTitle>New topic</DialogTitle>
              <DialogDescription>{CHAT_TOPIC_DESCRIPTION}</DialogDescription>
            </DialogHeader>
            <div className="finite-chat__rename-field">
              <label htmlFor="finite-chat-topic-title">Name</label>
              <Input
                id="finite-chat-topic-title"
                autoFocus
                maxLength={120}
                value={createTopicTitle}
                onChange={(event) => setCreateTopicTitle(event.target.value)}
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setCreateTopicOpen(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={busy || !canonicalRoomId || !createTopicTitle.trim()}>
                {busy ? "Creating…" : "Create topic"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={Boolean(renameTarget)} onOpenChange={(open) => {
        if (!open) setRenameTarget(null);
      }}>
        <DialogContent>
          <form className="finite-chat__rename-form" onSubmit={renameChat}>
            <DialogHeader>
              <DialogTitle>Rename chat</DialogTitle>
              <DialogDescription>Choose a name that makes this chat easy to find later.</DialogDescription>
            </DialogHeader>
            <div className="finite-chat__rename-field">
              <label htmlFor="finite-chat-sidebar-rename-title">Name</label>
              <Input
                id="finite-chat-sidebar-rename-title"
                autoFocus
                maxLength={120}
                value={renameTitle}
                onChange={(event) => setRenameTitle(event.target.value)}
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setRenameTarget(null)}>
                Cancel
              </Button>
              <Button type="submit" disabled={busy || !renameTitle.trim()}>
                {busy ? "Saving…" : "Save"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  );
}

function ChatRow({
  active,
  archived,
  chat,
  disabled,
  onArchiveChange,
  onOpen,
  onRename,
}: {
  active: boolean;
  archived: boolean;
  chat: HostedChatSummary;
  disabled: boolean;
  onArchiveChange?: (archived: boolean) => void;
  onOpen: () => void;
  onRename: () => void;
}) {
  const title = chat.title || "New chat";
  return (
    <div className={`finite-chat__thread-row ${active ? "is-active" : ""}`}>
      <button
        type="button"
        className="finite-chat__thread-open"
        aria-current={active ? "page" : undefined}
        onClick={onOpen}
      >
        <span className="finite-chat__thread-indicator" aria-hidden />
        <span className="finite-chat__thread-main">
          <span className="finite-chat__thread-title">{title}</span>
        </span>
      </button>
      <div className="finite-chat__thread-actions">
        {!archived ? (
          <button
            type="button"
            className="finite-chat__thread-action"
            aria-label={`Rename ${title}`}
            title="Rename chat"
            disabled={disabled}
            onClick={onRename}
          >
            <PencilIcon className="size-3.5" aria-hidden />
          </button>
        ) : null}
        {onArchiveChange ? (
          <button
            type="button"
            className="finite-chat__thread-action"
            aria-label={archived ? `Restore ${title}` : `Archive ${title}`}
            title={archived ? "Restore chat" : "Archive chat"}
            disabled={disabled}
            onClick={() => onArchiveChange(!archived)}
          >
            {archived
              ? <ArchiveRestoreIcon className="size-3.5" aria-hidden />
              : <ArchiveIcon className="size-3.5" aria-hidden />}
          </button>
        ) : null}
      </div>
    </div>
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

function safeDomId(value: string) {
  return value.replace(/[^a-zA-Z0-9_-]/gu, "-");
}
