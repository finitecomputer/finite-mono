import {
  type ComponentProps,
  type FormEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  ArrowDownIcon,
  ArrowUpIcon,
  ChevronRightIcon,
  CopyIcon,
  DownloadIcon,
  ExternalLinkIcon,
  FileTextIcon,
  HashIcon,
  Loader2Icon,
  MonitorIcon,
  MonitorSmartphoneIcon,
  MoreHorizontalIcon,
  PanelLeftIcon,
  PaperclipIcon,
  PencilIcon,
  PlusIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  Share2Icon,
  WrenchIcon,
  XIcon,
} from "lucide-react";

import type {
  AppAction,
  AppChatSummary,
  AppDeviceSummary,
  AppState,
  AppTopicSummary,
  ChatMediaAttachment,
  ChatMessage,
} from "../model";
import {
  activitiesForChat,
  attachmentSendError,
  beginPendingChatTurn,
  hasOutstandingPrincipalTurn,
  liveActivityLabel,
  messageContent,
  messagesForChat,
  pendingTurnIsComplete,
  pendingTurnMatchesSelection,
  selectedChat,
  topicsForRoom,
  transcriptItems,
  type ChatSelection,
  type PendingChatTurn,
} from "../presentation";
import {
  ChatProductController,
  errorMessage,
  type ChatProductControllerState,
} from "./controller";
import type { ChatProductLink, ChatProductPreview, ChatTransport } from "./transport";

const TYPING_IDLE_MS = 2_200;
const MAX_ATTACHMENT_BYTES = 32 * 1024 * 1024;
const MAX_ATTACHMENT_TOTAL_BYTES = 64 * 1024 * 1024;
const MAX_ATTACHMENTS = 8;
const AUTO_FOLLOW_SCROLL_THRESHOLD_PX = 120;

type PendingAttachment = {
  id: string;
  file: File;
  previewUrl: string | null;
};

type PublishedSite = { id: string; label: string; url: string };

export type ChatProductNavigation = {
  agent?: ChatProductLink | null;
  connections?: ChatProductLink | null;
  skills?: ChatProductLink | null;
  signOut?: ChatProductLink | null;
};

export type ChatProductProps = {
  transport: ChatTransport;
  machineLabel: string;
  viewerLabel?: string | null;
  initialDraft?: string;
  brand?: ReactNode;
  navigation?: ChatProductNavigation;
  preview?: ChatProductPreview;
  className?: string;
};

/**
 * The complete Finite Chat product surface. The two hosts provide transport,
 * identity labels, and optional navigation; everything a user can see and do
 * in a conversation is shared here.
 */
export function ChatProduct({
  brand,
  className,
  initialDraft,
  machineLabel,
  navigation,
  preview,
  transport,
  viewerLabel,
}: ChatProductProps) {
  const controller = useMemo(() => new ChatProductController(transport), [transport]);
  const [view, setView] = useState<ChatProductControllerState>(() => controller.snapshot());
  const [localError, setLocalError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [draft, setDraft] = useState(initialDraft ?? "");
  const [attachments, setAttachments] = useState<PendingAttachment[]>([]);
  const [pendingTurns, setPendingTurns] = useState<PendingChatTurn[]>([]);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [isDragOver, setIsDragOver] = useState(false);
  const [showLatest, setShowLatest] = useState(false);
  const [createTopicOpen, setCreateTopicOpen] = useState(false);
  const [createTopicTitle, setCreateTopicTitle] = useState("");
  const [renameOpen, setRenameOpen] = useState(false);
  const [renameTitle, setRenameTitle] = useState("");
  const [devicesOpen, setDevicesOpen] = useState(false);
  const [deviceToRevoke, setDeviceToRevoke] = useState<AppDeviceSummary | null>(null);
  const [deviceBusy, setDeviceBusy] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [activeSiteId, setActiveSiteId] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const typingRoomRef = useRef<string | null>(null);
  const typingTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const markedReadSeqRef = useRef(new Map<string, number>());
  const shouldFollowScrollRef = useRef(true);
  const attachmentsRef = useRef<PendingAttachment[]>([]);

  useEffect(() => controller.listen(setView), [controller]);
  useEffect(() => {
    controller.start();
    return () => controller.stop();
  }, [controller]);

  const state = view.state;
  const selection = useMemo(() => selectedChat(state, true), [state]);
  const topics = useMemo(
    () => topicsForRoom(state, selection.room?.room_id),
    [selection.room?.room_id, state]
  );
  const messages = useMemo(() => messagesForChat(state, selection), [selection, state]);
  const transcript = useMemo(
    () => transcriptItems(messages, state?.identity.account_id),
    [messages, state?.identity.account_id]
  );
  const activities = useMemo(() => activitiesForChat(state, selection), [selection, state]);
  const awaitingReply = selection.room?.is_agent_chat === true && (
    pendingTurns.some((turn) => pendingTurnMatchesSelection(turn, selection))
    || hasOutstandingPrincipalTurn(messages, state?.identity.account_id)
  );
  const agentLabel = friendlyRoomName(selection.room?.display_name, selection.room?.room_id)
    || state?.profiles.find((profile) => profile.is_agent)?.display_name
    || machineLabel;
  const viewerDisplayLabel = viewerLabel
    || state?.profiles.find((profile) => profile.account_id === state.identity.account_id)?.display_name
    || state?.identity.account_id
    || machineLabel;
  const activityLabel = liveActivityLabel(activities, agentLabel, awaitingReply);
  const sites = useMemo(() => sitesFromMessages(messages), [messages]);
  const activeSite = sites.find((site) => site.id === activeSiteId) ?? sites[0] ?? null;
  const connected = selection.room?.state === "Connected";
  const error = localError ?? view.error;

  useEffect(() => {
    if (!state) return;
    setPendingTurns((turns) => {
      const next = turns.filter(
        (turn) => !pendingTurnIsComplete(turn, state.messages, state.identity.account_id)
      );
      return next.length === turns.length ? turns : next;
    });
  }, [state]);

  useEffect(() => {
    if (sites.length === 0) {
      setPreviewOpen(false);
      setActiveSiteId(null);
    } else if (!activeSiteId || !sites.some((site) => site.id === activeSiteId)) {
      setActiveSiteId(sites[0]!.id);
    }
  }, [activeSiteId, sites]);

  useEffect(() => {
    const room = selection.room;
    if (!room || !state) return;
    const newestSeq = Math.max(
      0,
      ...state.messages.filter((message) => message.room_id === room.room_id).map((message) => message.seq)
    );
    if (newestSeq <= (markedReadSeqRef.current.get(room.room_id) ?? 0)) return;
    markedReadSeqRef.current.set(room.room_id, newestSeq);
    void dispatchQuiet(controller, { MarkRoomRead: { room_id: room.room_id } });
  }, [controller, selection.room, state]);

  const messagesFingerprint = useMemo(
    () => messages.map((message) => [
      message.message_id,
      message.kind,
      message.status,
      message.display_content,
      message.media?.length ?? 0,
    ].join(":" )).join("|"),
    [messages]
  );

  useLayoutEffect(() => {
    const node = scrollRef.current;
    if (node && shouldFollowScrollRef.current) node.scrollTop = node.scrollHeight;
  }, [activityLabel, messagesFingerprint, selection.chat?.chat_id]);

  useLayoutEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "0px";
    textarea.style.height = `${Math.min(textarea.scrollHeight, 220)}px`;
  }, [attachments.length, draft]);

  useEffect(() => {
    attachmentsRef.current = attachments;
  }, [attachments]);

  useEffect(
    () => () => attachmentsRef.current.forEach(revokeAttachmentPreview),
    []
  );

  const stopTyping = useCallback((roomId = typingRoomRef.current) => {
    if (typingTimerRef.current) clearTimeout(typingTimerRef.current);
    typingTimerRef.current = null;
    if (!roomId) return;
    typingRoomRef.current = null;
    void dispatchQuiet(controller, { SetTyping: { room_id: roomId, is_typing: false } });
  }, [controller]);

  useEffect(() => () => stopTyping(), [stopTyping]);

  function noteTyping(value: string) {
    const room = selection.room;
    if (!room || room.state !== "Connected") return;
    if (!value.trim()) {
      stopTyping(room.room_id);
      return;
    }
    if (typingRoomRef.current !== room.room_id) {
      typingRoomRef.current = room.room_id;
      void dispatchQuiet(controller, { SetTyping: { room_id: room.room_id, is_typing: true } });
    }
    if (typingTimerRef.current) clearTimeout(typingTimerRef.current);
    typingTimerRef.current = setTimeout(() => stopTyping(room.room_id), TYPING_IDLE_MS);
  }

  async function runAction(action: AppAction) {
    setLocalError(null);
    try {
      return await controller.dispatch(action);
    } catch (caught) {
      setLocalError(errorMessage(caught));
      return null;
    }
  }

  async function send(event: FormEvent) {
    event.preventDefault();
    const text = draft.trim();
    const room = selection.room;
    if ((!text && attachments.length === 0) || !room || sending) return;
    if (attachments.length > 0 && selection.topic && !selection.chat) {
      setLocalError("Start a chat in this topic before attaching files.");
      return;
    }
    setSending(true);
    setLocalError(null);
    stopTyping(room.room_id);
    const pendingTurn = beginPendingChatTurn(selection, messages);
    if (pendingTurn) {
      setPendingTurns((turns) => [
        ...turns.filter((turn) => !pendingTurnMatchesSelection(turn, selection)),
        pendingTurn,
      ]);
    }
    try {
      let next: AppState;
      if (attachments.length > 0) {
        next = await controller.upload({
          room_id: room.room_id,
          topic_id: selection.topic?.topic_id,
          chat_id: selection.chat?.chat_id,
          caption: text,
          files: attachments.map((attachment) => attachment.file),
        });
        const uploadError = attachmentSendError(next);
        if (uploadError) throw new Error(uploadError);
      } else {
        next = await controller.dispatch(messageAction(selection, text));
      }
      void next;
      setDraft("");
      setAttachments((current) => {
        current.forEach(revokeAttachmentPreview);
        return [];
      });
      requestAnimationFrame(() => textareaRef.current?.focus());
    } catch (caught) {
      if (pendingTurn) setPendingTurns((turns) => turns.filter((turn) => turn !== pendingTurn));
      setLocalError(errorMessage(caught));
    } finally {
      setSending(false);
    }
  }

  function addFiles(fileSource: FileList | File[]) {
    const incoming = Array.from(fileSource);
    const accepted = incoming.filter((file) => file.size <= MAX_ATTACHMENT_BYTES);
    const oversized = incoming.find((file) => file.size > MAX_ATTACHMENT_BYTES);
    if (oversized) setLocalError(`${oversized.name} is larger than ${formatBytes(MAX_ATTACHMENT_BYTES)}.`);
    setAttachments((current) => {
      const remaining = Math.max(0, MAX_ATTACHMENTS - current.length);
      const availableBytes = Math.max(
        0,
        MAX_ATTACHMENT_TOTAL_BYTES - current.reduce((total, attachment) => total + attachment.file.size, 0)
      );
      let selectedBytes = 0;
      const selected = accepted.slice(0, remaining).filter((file) => {
        if (selectedBytes + file.size > availableBytes) return false;
        selectedBytes += file.size;
        return true;
      });
      if (incoming.length > remaining) {
        setLocalError(`You can attach up to ${MAX_ATTACHMENTS} files at a time.`);
      } else if (selected.length < accepted.slice(0, remaining).length) {
        setLocalError(`Attachments can total up to ${formatBytes(MAX_ATTACHMENT_TOTAL_BYTES)}.`);
      }
      return [...current, ...selected.map((file) => ({
        id: attachmentId(file),
        file,
        previewUrl: file.type.startsWith("image/") ? URL.createObjectURL(file) : null,
      }))];
    });
  }

  async function openTopic(topic: AppTopicSummary) {
    if (await runAction({ OpenTopic: { room_id: topic.room_id, topic_id: topic.topic_id } })) {
      setSidebarOpen(false);
    }
  }

  async function openChat(topic: AppTopicSummary, chat: AppChatSummary) {
    if (await runAction({
      OpenChat: { room_id: topic.room_id, topic_id: topic.topic_id, chat_id: chat.chat_id },
    })) setSidebarOpen(false);
  }

  async function createChat(topic = selection.topic) {
    const room = selection.room;
    if (!room || !topic) return;
    if (await runAction({
      StartTopicChat: { room_id: room.room_id, topic_id: topic.topic_id, reason: null },
    })) setSidebarOpen(false);
  }

  async function createTopic(event: FormEvent) {
    event.preventDefault();
    const room = selection.room;
    const title = createTopicTitle.trim();
    if (!room || !title) return;
    if (await runAction({ CreateTopic: { room_id: room.room_id, title } })) {
      setCreateTopicOpen(false);
      setCreateTopicTitle("");
      setSidebarOpen(false);
    }
  }

  async function renameChat(event: FormEvent) {
    event.preventDefault();
    const { room, topic, chat } = selection;
    const title = renameTitle.trim();
    if (!room || !topic || !chat || !title) return;
    if (await runAction({
      RenameChat: {
        room_id: room.room_id,
        topic_id: topic.topic_id,
        chat_id: chat.chat_id,
        title,
      },
    })) setRenameOpen(false);
  }

  async function openDevices() {
    setDevicesOpen(true);
    setDeviceToRevoke(null);
    setDeviceBusy(true);
    await runAction({ RefreshDevices: null });
    setDeviceBusy(false);
  }

  async function revokeDevice() {
    if (!deviceToRevoke || deviceToRevoke.current_device || deviceToRevoke.revoked) return;
    setDeviceBusy(true);
    const next = await runAction({
      RevokeDevice: {
        account_id: deviceToRevoke.account_id,
        device_id: deviceToRevoke.device_id,
      },
    });
    if (next) setDeviceToRevoke(null);
    setDeviceBusy(false);
  }

  return (
    <div className={[
      "finite-chat-product",
      sidebarCollapsed ? "is-sidebar-collapsed" : "",
      className ?? "",
    ].filter(Boolean).join(" ")}>
      <ChatSidebar
        brand={brand}
        collapsed={sidebarCollapsed}
        isOpen={sidebarOpen}
        liveMembers={state?.typing_members ?? []}
        machineLabel={agentLabel}
        navigation={navigation}
        onCreateChat={(topic) => void createChat(topic)}
        onCreateTopic={() => setCreateTopicOpen(true)}
        onOpenChat={(topic, chat) => void openChat(topic, chat)}
        onOpenTopic={(topic) => void openTopic(topic)}
        onToggleCollapsed={() => setSidebarCollapsed((value) => !value)}
        onToggleOpen={() => setSidebarOpen((value) => !value)}
        selection={selection}
        topics={topics}
        viewerLabel={viewerDisplayLabel}
      />

      {sidebarOpen ? (
        <button
          type="button"
          className="finite-chat__sidebar-backdrop"
          aria-label="Close sidebar"
          onClick={() => setSidebarOpen(false)}
        />
      ) : null}

      <div className="finite-chat__workspace">
        <header className="finite-chat__topbar">
          <div className="finite-chat__breadcrumb">
            <button type="button" className="finite-chat__icon-button finite-chat__sidebar-toggle" aria-label="Open chats" onClick={() => setSidebarOpen(true)}>
              <PanelLeftIcon />
            </button>
            <span>{selection.topic?.title ?? "Home"}</span>
            <ChevronRightIcon />
            <strong>{selection.chat?.title ?? agentLabel}</strong>
            {selection.chat ? (
              <button type="button" className="finite-chat__icon-button finite-chat__rename-button" aria-label="Rename chat" onClick={() => {
                setRenameTitle(selection.chat?.title ?? "");
                setRenameOpen(true);
              }}><PencilIcon /></button>
            ) : null}
          </div>
          <div className="finite-chat__topbar-actions">
            {!view.streamConnected && state && transport.subscribe ? <span className="finite-chat__relay-warning">Reconnecting</span> : null}
            {navigation?.connections ? <NavLink link={navigation.connections} /> : null}
            <button type="button" className="finite-chat__topbar-button" onClick={() => void openDevices()}>
              <MonitorSmartphoneIcon /><span>Devices</span>
            </button>
            {preview && sites.length > 0 ? (
              <button type="button" className="finite-chat__topbar-button" aria-expanded={previewOpen} onClick={() => setPreviewOpen((value) => !value)}>
                <MonitorIcon /><span>Preview</span>
              </button>
            ) : null}
          </div>
        </header>

        <div className={previewOpen && activeSite ? "finite-chat__split has-preview" : "finite-chat__split"}>
          <section className="finite-chat__main" aria-label="Chat">
            <div
              className="finite-chat__scroll"
              ref={scrollRef}
              onScroll={(event) => {
                const node = event.currentTarget;
                const distance = node.scrollHeight - node.scrollTop - node.clientHeight;
                shouldFollowScrollRef.current = distance <= AUTO_FOLLOW_SCROLL_THRESHOLD_PX;
                setShowLatest(!shouldFollowScrollRef.current);
              }}
            >
              {!state && view.loading ? <ChatLoading label="Opening your chat…" /> : null}
              {state && !selection.room ? <EmptyChat title="Connecting to your agent" body="Your chat is getting ready." /> : null}
              {selection.room && messages.length === 0 && !activityLabel ? (
                <EmptyChat title="What should we work on?" body="Start here, or make a new chat inside this topic." />
              ) : null}
              {messages.length > 0 || activityLabel ? (
                <div className="finite-chat__messages" aria-live="polite">
                  {selection.room?.can_load_older && messages[0] ? (
                    <button type="button" className="finite-chat__load-older-button" onClick={() => void runAction({
                      LoadOlderMessages: {
                        room_id: selection.room!.room_id,
                        before_message_id: messages[0]!.message_id,
                        limit: 80,
                      },
                    })}>Load earlier messages</button>
                  ) : null}
                  {transcript.map((item) => item.type === "message" ? (
                    <MessageRow key={item.message.message_id} message={item.message} ownAccountId={state?.identity.account_id ?? ""} transport={transport} />
                  ) : (
                    <ToolRollup key={item.id} messages={item.messages} />
                  ))}
                  {activityLabel ? <LiveActivity label={activityLabel} /> : null}
                </div>
              ) : null}
            </div>

            {showLatest ? (
              <button type="button" className="finite-chat__scroll-bottom-button" onClick={() => {
                scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
                shouldFollowScrollRef.current = true;
                setShowLatest(false);
              }}><ArrowDownIcon /><span>Latest</span></button>
            ) : null}

            {error ? (
              <div className="finite-chat__send-error" role="alert">
                <strong>Chat needs attention</strong><span>{error}</span>
                <button type="button" onClick={() => {
                  setLocalError(null);
                  void controller.refresh().catch(() => undefined);
                }}><RotateCcwIcon /> Retry</button>
              </div>
            ) : null}

            <div className="finite-chat__composer-wrap">
              <form
                className={`finite-chat__composer ${isDragOver ? "is-drag-over" : ""}`}
                onSubmit={send}
                onDragOver={(event) => { event.preventDefault(); setIsDragOver(true); }}
                onDragLeave={() => setIsDragOver(false)}
                onDrop={(event) => {
                  event.preventDefault();
                  setIsDragOver(false);
                  addFiles(event.dataTransfer.files);
                }}
              >
                {attachments.length > 0 ? (
                  <div className="finite-chat__attachments">
                    {attachments.map((attachment) => (
                      <div key={attachment.id} className="finite-chat__attachment-chip">
                        {attachment.previewUrl ? <img src={attachment.previewUrl} alt="" /> : <FileTextIcon />}
                        <span>{attachment.file.name}</span>
                        <button type="button" aria-label={`Remove ${attachment.file.name}`} onClick={() => setAttachments((current) => {
                          const removed = current.find((candidate) => candidate.id === attachment.id);
                          if (removed) revokeAttachmentPreview(removed);
                          return current.filter((candidate) => candidate.id !== attachment.id);
                        })}><XIcon /></button>
                      </div>
                    ))}
                  </div>
                ) : null}
                <textarea
                  ref={textareaRef}
                  aria-label="Message your agent"
                  placeholder={connected ? `Ask ${agentLabel} anything` : "Waiting for your agent to connect…"}
                  value={draft}
                  disabled={!connected || sending}
                  rows={1}
                  onBlur={() => stopTyping(selection.room?.room_id)}
                  onChange={(event) => { setDraft(event.target.value); noteTyping(event.target.value); }}
                  onPaste={(event) => {
                    if (event.clipboardData.files.length > 0) addFiles(event.clipboardData.files);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" && !event.shiftKey) {
                      event.preventDefault();
                      event.currentTarget.form?.requestSubmit();
                    }
                  }}
                />
                <div className="finite-chat__composer-actions">
                  <input ref={fileInputRef} type="file" hidden multiple onChange={(event) => {
                    if (event.currentTarget.files) addFiles(event.currentTarget.files);
                    event.currentTarget.value = "";
                  }} />
                  <button type="button" className="finite-chat__tool-button" disabled={!connected || sending || !transport.upload} aria-label="Attach files" onClick={() => fileInputRef.current?.click()}>
                    <PaperclipIcon />
                  </button>
                  <button type="submit" className="finite-chat__send-button" aria-label="Send message" disabled={!connected || (!draft.trim() && attachments.length === 0) || sending}>
                    {sending ? <Loader2Icon className="finite-chat__spin" /> : <ArrowUpIcon />}
                  </button>
                </div>
              </form>
            </div>
          </section>

          {previewOpen && activeSite && preview ? (
            <SitePreviewPanel site={activeSite} preview={preview} sites={sites} onClose={() => setPreviewOpen(false)} onSelect={setActiveSiteId} />
          ) : null}
        </div>
      </div>

      {createTopicOpen ? (
        <Modal title="New topic" description="Topics keep related chats together." onClose={() => setCreateTopicOpen(false)}>
          <form className="finite-chat__modal-form" onSubmit={createTopic}>
            <label>Name<input autoFocus maxLength={120} value={createTopicTitle} onChange={(event) => setCreateTopicTitle(event.target.value)} /></label>
            <div className="finite-chat__modal-actions"><button type="button" onClick={() => setCreateTopicOpen(false)}>Cancel</button><button type="submit" disabled={!createTopicTitle.trim()}>Create topic</button></div>
          </form>
        </Modal>
      ) : null}

      {renameOpen ? (
        <Modal title="Rename chat" description="Choose a name that makes this chat easy to find later." onClose={() => setRenameOpen(false)}>
          <form className="finite-chat__modal-form" onSubmit={renameChat}>
            <label>Name<input autoFocus maxLength={120} value={renameTitle} onChange={(event) => setRenameTitle(event.target.value)} /></label>
            <div className="finite-chat__modal-actions"><button type="button" onClick={() => setRenameOpen(false)}>Cancel</button><button type="submit" disabled={!renameTitle.trim()}>Save</button></div>
          </form>
        </Modal>
      ) : null}

      {devicesOpen ? (
        <Modal
          title={deviceToRevoke ? `Revoke ${deviceToRevoke.device_id}?` : "Your devices"}
          description={deviceToRevoke
            ? "This permanently stops that Device from sending, receiving, or linking again with the same Device identity."
            : "Each linked browser or computer is a separate encrypted Device."}
          onClose={() => { setDevicesOpen(false); setDeviceToRevoke(null); }}
        >
          {deviceToRevoke ? (
            <div className="finite-chat__modal-actions"><button type="button" disabled={deviceBusy} onClick={() => setDeviceToRevoke(null)}>Cancel</button><button type="button" className="is-danger" disabled={deviceBusy} onClick={() => void revokeDevice()}>{deviceBusy ? "Revoking…" : "Revoke Device"}</button></div>
          ) : (
            <div className="finite-chat__device-list">
              {deviceBusy ? <ChatLoading label="Refreshing devices…" /> : null}
              {!deviceBusy && (state?.devices ?? []).length === 0 ? <p>No linked Devices are visible yet.</p> : null}
              {(state?.devices ?? []).map((device) => (
                <div key={`${device.account_id}:${device.device_id}`} className="finite-chat__device-row">
                  <MonitorSmartphoneIcon />
                  <span><strong>{device.device_id}</strong><small>{device.current_device ? "This Device" : device.revoked ? "Revoked" : device.active ? `Active in ${device.room_count} ${pluralize("room", device.room_count)}` : "Linked, not currently active"}</small></span>
                  {!device.current_device && !device.revoked ? <button type="button" onClick={() => setDeviceToRevoke(device)}>Revoke</button> : null}
                </div>
              ))}
            </div>
          )}
        </Modal>
      ) : null}
    </div>
  );
}

function ChatSidebar({
  brand,
  collapsed,
  isOpen,
  liveMembers,
  machineLabel,
  navigation,
  onCreateChat,
  onCreateTopic,
  onOpenChat,
  onOpenTopic,
  onToggleCollapsed,
  onToggleOpen,
  selection,
  topics,
  viewerLabel,
}: {
  brand?: ReactNode;
  collapsed: boolean;
  isOpen: boolean;
  liveMembers: AppState["typing_members"];
  machineLabel: string;
  navigation?: ChatProductNavigation;
  onCreateChat: (topic: AppTopicSummary) => void;
  onCreateTopic: () => void;
  onOpenChat: (topic: AppTopicSummary, chat: AppChatSummary) => void;
  onOpenTopic: (topic: AppTopicSummary) => void;
  onToggleCollapsed: () => void;
  onToggleOpen: () => void;
  selection: ChatSelection;
  topics: AppTopicSummary[];
  viewerLabel?: string | null;
}) {
  return (
    <aside className={`finite-chat__sidebar ${isOpen ? "is-open" : ""}`} aria-hidden={collapsed || undefined}>
      <div className="finite-chat__sidebar-top">
        <div className="finite-chat__brand">{brand ?? <DefaultBrand />}</div>
        <button type="button" className="finite-chat__icon-button finite-chat__desktop-collapse-button" aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"} onClick={onToggleCollapsed}><PanelLeftIcon /></button>
        <button type="button" className="finite-chat__icon-button finite-chat__mobile-collapse-button" aria-label="Close sidebar" onClick={onToggleOpen}><PanelLeftIcon /></button>
      </div>
      <nav className="finite-chat__sidebar-nav" aria-label="Topics and chats">
        <div className="finite-chat__sidebar-section-row"><span>Topics</span><button type="button" className="finite-chat__icon-button" aria-label="New topic" onClick={onCreateTopic}><PlusIcon /></button></div>
        {topics.map((topic) => (
          <div className="finite-chat__folder" key={topic.topic_id}>
            <div className="finite-chat__folder-header">
              <button type="button" className={`finite-chat__folder-summary ${topic.topic_id === selection.topic?.topic_id ? "is-active" : ""}`} onClick={() => onOpenTopic(topic)}>
                <HashIcon /><span>{topic.title}</span>{topic.unread_count > 0 ? <b>{topic.unread_count}</b> : null}
              </button>
              <button type="button" className="finite-chat__icon-button" aria-label={`New chat in ${topic.title}`} onClick={() => onCreateChat(topic)}><PlusIcon /></button>
            </div>
            <div className="finite-chat__folder-body">
              {topic.chats.map((chat) => {
                const activity = liveMembers.find((member) =>
                  (!member.topic_id || member.topic_id === topic.topic_id)
                  && (!member.chat_id || member.chat_id === chat.chat_id)
                )?.activity_kind;
                const active = topic.topic_id === selection.topic?.topic_id && chat.chat_id === selection.chat?.chat_id;
                return (
                  <button key={chat.chat_id} type="button" className={`${active ? "is-active" : ""} ${activity ? "is-working" : ""}`} aria-current={active ? "page" : undefined} aria-busy={activity ? true : undefined} onClick={() => onOpenChat(topic, chat)}>
                    <span className={`finite-chat__thread-indicator ${activity ? `is-${activity}` : ""}`}>{activity ? <i /> : null}</span>
                    <span>{chat.title || "New chat"}</span>{chat.unread_count > 0 ? <b>{chat.unread_count}</b> : null}
                  </button>
                );
              })}
            </div>
          </div>
        ))}
      </nav>
      <button type="button" className="finite-chat__sidebar-new-chat" disabled={!selection.topic} onClick={() => selection.topic && onCreateChat(selection.topic)}><PlusIcon /><span>New chat</span></button>
      <details className="finite-chat__account-menu">
        <summary><span className="finite-chat__avatar">{initials(viewerLabel || machineLabel)}</span><span>{viewerLabel || machineLabel}</span><MoreHorizontalIcon /></summary>
        <div>
          {navigation?.agent ? <NavLink link={navigation.agent} /> : null}
          {navigation?.connections ? <NavLink link={navigation.connections} /> : null}
          {navigation?.skills ? <NavLink link={navigation.skills} /> : null}
          {navigation?.signOut ? <NavLink link={navigation.signOut} /> : null}
        </div>
      </details>
    </aside>
  );
}

function MessageRow({ message, ownAccountId, transport }: { message: ChatMessage; ownAccountId: string; transport: ChatTransport }) {
  const mine = message.sender_account_id === ownAccountId || (!ownAccountId && message.is_mine);
  const content = messageContent(message);
  return (
    <article className={`finite-chat__message ${mine ? "finite-chat__message--user" : "finite-chat__message--agent"}`}>
      <div>
        <MessageAttachments message={message} transport={transport} compact={mine} />
        {content ? mine ? <p>{content}</p> : <MarkdownMessage text={content} /> : null}
        <time className="finite-chat__message-time">{mine ? deliveryText(message) || message.display_timestamp : message.display_timestamp}</time>
      </div>
    </article>
  );
}

function MessageAttachments({ message, transport, compact }: { message: ChatMessage; transport: ChatTransport; compact: boolean }) {
  if (!message.media?.length) return null;
  return (
    <div className={`finite-chat__media-grid ${compact ? "is-compact" : ""}`}>
      {message.media.map((attachment) => <AttachmentCard key={attachment.attachment_id} attachment={attachment} message={message} transport={transport} compact={compact} />)}
    </div>
  );
}

function AttachmentCard({ attachment, compact, message, transport }: { attachment: ChatMediaAttachment; compact: boolean; message: ChatMessage; transport: ChatTransport }) {
  const href = transport.attachmentUrl?.({ room_id: message.room_id, message_id: message.message_id, attachment_id: attachment.attachment_id }) ?? attachment.url ?? null;
  if (!href) return <span className="finite-chat__file-card"><FileTextIcon /><span>{attachment.filename}</span></span>;
  if (attachment.kind !== "Image") return <a href={href} className="finite-chat__file-card" download={attachment.filename}><FileTextIcon /><span>{attachment.filename}</span></a>;
  return (
    <span className={`finite-chat__image-card ${compact ? "is-compact" : ""}`}>
      <a href={href} target="_blank" rel="noreferrer"><img src={href} alt={attachment.filename} /></a>
      <span className="finite-chat__image-caption"><span>{attachment.filename}</span><span><a href={href} download={attachment.filename} aria-label={`Download ${attachment.filename}`}><DownloadIcon /></a><ShareAttachmentButton href={href} name={attachment.filename} /></span></span>
    </span>
  );
}

function ShareAttachmentButton({ href, name }: { href: string; name: string }) {
  if (typeof navigator === "undefined" || !("share" in navigator)) return null;
  return <button type="button" aria-label={`Share ${name}`} onClick={() => void navigator.share({ title: name, url: new URL(href, window.location.href).toString() }).catch(() => undefined)}><Share2Icon /></button>;
}

function ToolRollup({ messages }: { messages: ChatMessage[] }) {
  const running = messages.some((message) => message.status === "running");
  const steps = messages.flatMap((message) => messageContent(message).split(/\n+/u).filter(Boolean));
  const count = steps.length || messages.length;
  return (
    <details className="finite-chat__tool-rollup" open={running || undefined}>
      <summary>{running ? <Loader2Icon className="finite-chat__spin" /> : <WrenchIcon />}<span>{running ? `Working · ${count} ${pluralize("step", count)}` : `Worked through ${count} ${pluralize("step", count)}`}</span><ChevronRightIcon /></summary>
      <div>{messages.map((message) => <pre key={message.message_id}>{messageContent(message) || "Done"}</pre>)}</div>
    </details>
  );
}

function LiveActivity({ label }: { label: string }) {
  return <div className="finite-chat__live-activity" aria-live="polite"><span><i /><i /><i /></span><span>{label}</span></div>;
}

function MarkdownMessage({ text }: { text: string }) {
  return <div className="finite-chat__assistant-text finite-chat__markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} components={{ a: MarkdownLink, table: MarkdownTable }}>{text}</ReactMarkdown></div>;
}

function MarkdownTable({ children, ...props }: ComponentProps<"table">) {
  return <div className="finite-chat__table-scroll"><table {...props}>{children}</table></div>;
}

function MarkdownLink({ children, href }: ComponentProps<"a">) {
  return <a href={typeof href === "string" ? href : ""} target="_blank" rel="noreferrer">{children}</a>;
}

function SitePreviewPanel({ onClose, onSelect, preview, site, sites }: { onClose: () => void; onSelect: (id: string) => void; preview: ChatProductPreview; site: PublishedSite; sites: PublishedSite[] }) {
  const [frameUrl, setFrameUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [reload, setReload] = useState(0);
  useEffect(() => {
    let disposed = false;
    setFrameUrl(null);
    setFailed(false);
    preview.createSession(site.url).then((url) => {
      if (!disposed) setFrameUrl(url);
    }).catch(() => {
      if (!disposed) setFailed(true);
    });
    return () => { disposed = true; };
  }, [preview, reload, site.url]);
  return (
    <aside className="finite-chat__preview" aria-label="Site preview">
      <div className="finite-chat__browser-chrome">
        <span className="finite-chat__traffic-lights"><i /><i /><i /></span>
        {sites.length > 1 ? <select aria-label="Select site preview" value={site.id} onChange={(event) => onSelect(event.target.value)}>{sites.map((candidate) => <option key={candidate.id} value={candidate.id}>{candidate.label}</option>)}</select> : <strong>{site.label}</strong>}
        <input aria-label="Preview URL" readOnly value={site.url} />
        <button type="button" aria-label="Copy preview link" onClick={() => void navigator.clipboard.writeText(site.url)}><CopyIcon /></button>
        <a href={site.url} target="_blank" rel="noreferrer" aria-label="Open preview"><ExternalLinkIcon /></a>
        <button type="button" aria-label="Reload preview" onClick={() => setReload((value) => value + 1)}><RefreshCwIcon /></button>
        <button type="button" aria-label="Close preview" onClick={onClose}><XIcon /></button>
      </div>
      <div className="finite-chat__browser-viewport">{frameUrl ? <iframe key={`${site.id}:${reload}`} src={frameUrl} title={site.label} sandbox="allow-forms allow-same-origin allow-scripts" referrerPolicy="no-referrer" /> : failed ? <div className="finite-chat__notice">Preview isn&apos;t available right now.</div> : <ChatLoading label="Opening preview…" />}</div>
    </aside>
  );
}

function Modal({ children, description, onClose, title }: { children: ReactNode; description: string; onClose: () => void; title: string }) {
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  useEffect(() => {
    const previousFocus = document.activeElement;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onCloseRef.current();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      if (previousFocus instanceof HTMLElement) previousFocus.focus();
    };
  }, []);
  return <div className="finite-chat__modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.currentTarget === event.target) onClose(); }}><section className="finite-chat__modal" role="dialog" aria-modal="true" aria-label={title}><button type="button" className="finite-chat__modal-close" aria-label="Close" onClick={onClose}><XIcon /></button><h2>{title}</h2><p>{description}</p>{children}</section></div>;
}

function NavLink({ link }: { link: ChatProductLink }) {
  if (link.onClick) {
    return <button type="button" className="finite-chat__nav-link" onClick={() => void link.onClick?.()}>{link.label}</button>;
  }
  return <a className="finite-chat__nav-link" href={link.href ?? "#"}>{link.label}</a>;
}

function DefaultBrand() {
  return <span className="finite-chat__default-brand"><i />Finite.Computer</span>;
}

function EmptyChat({ body, title }: { body: string; title: string }) {
  return <div className="finite-chat__empty"><span /><h1>{title}</h1><p>{body}</p></div>;
}

function ChatLoading({ label }: { label: string }) {
  return <div className="finite-chat__notice"><Loader2Icon className="finite-chat__spin" /><span>{label}</span></div>;
}

function messageAction(selection: ChatSelection, text: string): AppAction {
  const roomId = selection.room!.room_id;
  if (selection.topic && selection.chat) return { SendChatMessage: { room_id: roomId, topic_id: selection.topic.topic_id, chat_id: selection.chat.chat_id, text } };
  if (selection.topic) return { SendTopicMessage: { room_id: roomId, topic_id: selection.topic.topic_id, text } };
  return { SendMessage: { room_id: roomId, text } };
}

async function dispatchQuiet(controller: ChatProductController, action: AppAction) {
  try { return await controller.dispatch(action); } catch { return null; }
}

function sitesFromMessages(messages: ChatMessage[]) {
  const seen = new Set<string>();
  const sites: PublishedSite[] = [];
  const pattern = /https?:\/\/[^\s<>()\[\]{}"']+/giu;
  for (const message of [...messages].reverse()) {
    for (const raw of messageContent(message).match(pattern) ?? []) {
      const value = raw.replace(/[.,;:!?]+$/u, "");
      try {
        const url = new URL(value);
        const allowed = url.hostname.endsWith(".localhost") || url.hostname.endsWith(".finite.chat");
        if (!allowed || /^(?:api|git)\./u.test(url.hostname) || url.pathname.endsWith(".git") || seen.has(url.toString())) continue;
        seen.add(url.toString());
        sites.push({ id: url.toString(), label: url.hostname, url: url.toString() });
      } catch { /* Ignore malformed prose URLs. */ }
    }
  }
  return sites.slice(0, 8);
}

function deliveryText(message: ChatMessage) {
  const delivery = message.outbound_delivery;
  if (!delivery) return null;
  if (typeof delivery.server_delivery === "object" && "Failed" in delivery.server_delivery) return "Not delivered";
  if (delivery.server_delivery === "Undelivered") return "Sending…";
  return "Delivered";
}

function attachmentId(file: File) {
  const random = typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : Math.random().toString(36).slice(2);
  return `${file.name}-${file.size}-${file.lastModified}-${random}`;
}

function revokeAttachmentPreview(attachment: PendingAttachment) {
  if (attachment.previewUrl) URL.revokeObjectURL(attachment.previewUrl);
}

function initials(value: string) {
  const parts = value.split(/[@._\-\s]+/u).filter(Boolean);
  if (parts.length === 0) return "FC";
  if (parts.length === 1) return parts[0]!.slice(0, 2).toUpperCase();
  return `${parts[0]![0] ?? "F"}${parts[1]![0] ?? "C"}`.toUpperCase();
}

function pluralize(word: string, count: number) {
  return count === 1 ? word : `${word}s`;
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function friendlyRoomName(displayName?: string | null, roomId?: string | null) {
  const value = displayName?.trim();
  if (!value || value === roomId || /^room[-_:]/iu.test(value)) return null;
  return value;
}
