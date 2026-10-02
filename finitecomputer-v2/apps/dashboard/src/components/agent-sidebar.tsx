"use client";

import type { CSSProperties, DragEvent, FormEvent, ReactNode } from "react";
import { useCallback, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { usePathname, useRouter } from "next/navigation";
import {
  ArchiveIcon,
  ArchiveRestoreIcon,
  ChevronRightIcon,
  HashIcon,
  GripVerticalIcon,
  LogInIcon,
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
import { ChatMoveDialog } from "@/components/chat-move-dialog";
import { Input } from "@/components/ui/input";
import { CHAT_TOPIC_DESCRIPTION } from "@/lib/chat-product-copy";
import type {
  HostedChatAction,
  HostedChatTopic,
} from "@/lib/hosted-web-device";
import { sidebarTopics, sidebarChatKey, type SidebarChat, canonicalNewChatTopic, HOME_TOPIC_ID } from "@/lib/hosted-web-chat-topics";

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
    sessionError,
    bindingRecoveryRequired,
    load,
    recoverBinding,
    reportSessionAuthFailure,
    signInAgain,
    dispatch,
  } = useHostedChat();
  const hydrated = useSyncExternalStore(
    subscribeHydration,
    () => true,
    () => false
  );
  // The hosted web runtime always supports the durable chat archive.
  const supportsChatArchive = hydrated;
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
    chat: SidebarChat;
  } | null>(null);
  const [renameTitle, setRenameTitle] = useState("");
  const [moveTarget, setMoveTarget] = useState<SidebarChat | null>(null);
  const [draggedChat, setDraggedChat] = useState<SidebarChat | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const moving = useRef(false);
  const [moveStatus, setMoveStatus] = useState("");

  const canonicalRoomId = state?.hosted_agent_binding?.canonical_room_id ?? null;
  const topics = useMemo(
    () => sidebarTopics((state?.topics ?? [])
      .filter((topic) => topic.room_id === canonicalRoomId && !topic.archived)
      .sort((left, right) => {
        if (left.topic_id === HOME_TOPIC_ID) return -1;
        if (right.topic_id === HOME_TOPIC_ID) return 1;
        return right.updated_seq - left.updated_seq || left.title.localeCompare(right.title);
      })),
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
      if (reportSessionAuthFailure(caught)) return null;
      setActionError(caught instanceof Error
        ? caught.message
        : "That chat action is temporarily unavailable.");
      return null;
    } finally {
      setBusy(false);
    }
  }, [dispatch, machineId, onMobileOpenChange, pathname, reportSessionAuthFailure, router]);

  function openChat(topic: HostedChatTopic, chat: SidebarChat) {
    void act({
      OpenChat: {
        room_id: topic.room_id,
        topic_id: chat.source_topic_id,
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
    chat: SidebarChat,
    archived: boolean
  ) {
    if (archived) {
      const topicKey = `${topic.room_id}:${topic.topic_id}`;
      setExpandedArchiveKeys((current) => new Set(current).add(topicKey));
    }
    void act({
      SetChatArchived: {
        room_id: topic.room_id,
        topic_id: chat.source_topic_id,
        chat_id: chat.chat_id,
        archived,
      },
    });
  }

  function openRename(topic: HostedChatTopic, chat: SidebarChat) {
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
        topic_id: renameTarget.chat.source_topic_id,
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

  async function moveChat(chat: SidebarChat, destinationTopicId: string, before: SidebarChat | null) {
    if (busy || moving.current || !canonicalRoomId || !chat.placement) return false;
    const source = topics.flatMap((topic) => topic.chats).find((item) => sidebarChatKey(item) === sidebarChatKey(chat));
    const destination = topics.find((topic) => topic.topic_id === destinationTopicId);
    if (!source || source.archived || !destination || (before &&
      (sidebarChatKey(before) === sidebarChatKey(source) || !destination.chats.some((item) =>
        !item.archived && sidebarChatKey(item) === sidebarChatKey(before))))) return false;
    moving.current = true;
    const next = await act({ MoveChat: {
      room_id: canonicalRoomId, topic_id: source.source_topic_id, chat_id: source.chat_id,
      destination_topic_id: destinationTopicId,
      before: before ? { topic_id: before.source_topic_id, chat_id: before.chat_id } : null,
    } });
    moving.current = false;
    if (!next) { void load(false); return false; }
    setCollapsedTopicKeys((current) => {
      const expanded = new Set(current);
      expanded.delete(`${canonicalRoomId}:${destinationTopicId}`);
      return expanded;
    });
    setMoveStatus(`${source.title || "Chat"} moved to ${destination.title}.`);
    return true;
  }

  function dragOver(event: DragEvent, topicId: string, before: SidebarChat | null) {
    event.stopPropagation();
    if (!draggedChat || busy || (before && sidebarChatKey(before) === sidebarChatKey(draggedChat))) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    setDropTarget(JSON.stringify([topicId, before ? sidebarChatKey(before) : null]));
  }

  function dropChat(event: DragEvent, topicId: string, before: SidebarChat | null) {
    event.preventDefault();
    event.stopPropagation();
    const chat = draggedChat;
    setDraggedChat(null);
    setDropTarget(null);
    if (chat) void moveChat(chat, topicId, before);
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
          {!state && !transportError && !sessionError ? <p className="finite-agent-sidebar__status">Loading chats…</p> : null}
          {sessionError ? (
            <div className="finite-agent-sidebar__error">
              <span>{sessionError}</span>
              <Button type="button" variant="ghost" size="sm" onClick={signInAgain}>
                <LogInIcon />
                Sign in again
              </Button>
            </div>
          ) : null}
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
            <div className="finite-agent-sidebar__error" role="alert">
              <span>{actionError}</span>
              <Button type="button" variant="ghost" size="sm" onClick={() => setActionError(null)}>
                Dismiss
              </Button>
            </div>
          ) : null}
          <span className="sr-only" role="status">{moveStatus}</span>
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
              (chat) => chat.source_topic_id === selectedTopicId && chat.chat_id === selectedChatId
            );
            const archiveExpanded =
              expandedArchiveKeys.has(topicKey) || selectedChatIsArchived;
            return (
              <div className="finite-chat__folder" key={topicKey}
                data-topic-id={topic.topic_id}
                data-drop-target={dropTarget === JSON.stringify([topic.topic_id, null]) || undefined}
                onDragOver={(event) => dragOver(event, topic.topic_id, null)}
                onDragLeave={(event) => {
                  if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDropTarget(null);
                }}
                onDrop={(event) => dropChat(event, topic.topic_id, null)}
              >
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
                      key={sidebarChatKey(chat)}
                      active={chat.source_topic_id === selectedTopicId && chat.chat_id === selectedChatId}
                      archived={false}
                      dropTarget={dropTarget === JSON.stringify([topic.topic_id, sidebarChatKey(chat)])}
                      onDragOver={(event) => dragOver(event, topic.topic_id, chat)}
                      onDrop={(event) => dropChat(event, topic.topic_id, chat)}
                      onDragStart={(event) => {
                        event.dataTransfer.effectAllowed = "move";
                        event.dataTransfer.setData("application/x-finite-chat", sidebarChatKey(chat));
                        setDraggedChat(chat);
                      }}
                      onDragEnd={() => { setDraggedChat(null); setDropTarget(null); }}
                      onMove={chat.placement ? () => setMoveTarget(chat) : undefined}
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
                            key={sidebarChatKey(chat)}
                            active={chat.source_topic_id === selectedTopicId && chat.chat_id === selectedChatId}
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

      <ChatMoveDialog chat={moveTarget} topics={topics} busy={busy}
        onClose={() => setMoveTarget(null)} onMove={moveChat} />
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
  onMove, onDragStart, onDragEnd, onDragOver, onDrop, dropTarget,
}: {
  active: boolean;
  archived: boolean;
  chat: SidebarChat;
  disabled: boolean;
  onArchiveChange?: (archived: boolean) => void;
  onOpen: () => void;
  onRename: () => void;
  onMove?: () => void;
  onDragStart?: (event: DragEvent) => void;
  onDragEnd?: () => void;
  onDragOver?: (event: DragEvent) => void;
  onDrop?: (event: DragEvent) => void;
  dropTarget?: boolean;
}) {
  const title = chat.title || "New chat";
  return (
    <div className={`finite-chat__thread-row ${active ? "is-active" : ""}`}
      data-chat-id={chat.chat_id} data-source-topic-id={chat.source_topic_id}
      data-drop-target={dropTarget || undefined} onDragOver={onDragOver} onDrop={onDrop}>
      {onMove ? <button type="button" className="finite-chat__thread-action finite-chat__drag-handle"
        aria-label={`Move ${title}`} title="Drag to move, or click for options"
        draggable={!disabled} disabled={disabled} onClick={onMove}
        onDragStart={onDragStart} onDragEnd={onDragEnd}>
        <GripVerticalIcon className="size-3.5" aria-hidden />
      </button> : null}
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
