"use client";

import type { ComponentProps } from "react";
import {
  FormEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import ReactMarkdown from "react-markdown";
import { Group, Panel, Separator } from "react-resizable-panels";
import remarkGfm from "remark-gfm";
import { Drawer } from "vaul";
import {
  ArrowDownIcon,
  ArrowUpIcon,
  ChevronRightIcon,
  CopyIcon,
  DownloadIcon,
  ExternalLinkIcon,
  FileTextIcon,
  Loader2Icon,
  MonitorIcon,
  PanelLeftIcon,
  PaperclipIcon,
  PencilIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  Share2Icon,
  WrenchIcon,
  XIcon,
} from "lucide-react";

import {
  CHAT_WAITING_FOR_AGENT_MESSAGE,
} from "@/lib/chat-product-copy";
import {
  hostedChatErrorMessage,
  useHostedChat,
} from "@/components/hosted-chat-provider";
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
import type {
  HostedChatAction,
  HostedChatMediaAttachment,
  HostedChatMessage,
  HostedChatState,
  HostedChatSummary,
  HostedChatTopic,
} from "@/lib/hosted-web-device";
import { HOME_TOPIC_ID } from "@/lib/hosted-web-chat-topics";
import {
  activityLeaseIsFresh,
  beginPendingChatTurn,
  attachmentSendError,
  liveActivityLabel as sharedLiveActivityLabel,
  messageContent,
  pendingTurnIsComplete,
  pendingTurnLeaseIsFresh,
  pendingTurnMatchesSelection,
  transcriptItems,
  type ChatSelection,
  type PendingChatTurn,
} from "@finite/chat-ui";

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

type PreviewSite = {
  id: string;
  label: string;
  url: string;
};

export function HostedWebChat({
  initialDraft,
  machineId,
  machineLabel,
}: {
  initialDraft?: string;
  machineId: string;
  machineLabel: string;
}) {
  const {
    apiBase,
    state,
    transportError,
    claimError,
    bindingRecoveryRequired,
    streamConnected,
    ownerClaimed,
    load,
    claimOwner,
    recoverBinding,
    dispatch,
    dispatchQuiet,
    uploadAttachments,
  } = useHostedChat();
  const [actionError, setActionError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [draft, setDraft] = useState(initialDraft ?? "");
  const [attachments, setAttachments] = useState<PendingAttachment[]>([]);
  const [pendingAgentTurns, setPendingAgentTurns] = useState<PendingChatTurn[]>([]);
  const [activityObservedAtMs, setActivityObservedAtMs] = useState<number | null>(null);
  const [leaseNowMs, setLeaseNowMs] = useState(() => Date.now());
  const [isDragOver, setIsDragOver] = useState(false);
  const [renameOpen, setRenameOpen] = useState(false);
  const [renameTitle, setRenameTitle] = useState("");
  const [browserOpen, setBrowserOpen] = useState(false);
  const [activeSiteId, setActiveSiteId] = useState<string | null>(null);
  const [showLatest, setShowLatest] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const typingRoomRef = useRef<string | null>(null);
  const typingTimerRef = useRef<number | null>(null);
  const latestSiteIdRef = useRef<string | null>(null);
  const shouldFollowScrollRef = useRef(true);
  const attachmentsRef = useRef<PendingAttachment[]>([]);
  const markedReadSeqRef = useRef(new Map<string, number>());
  const mobilePreview = useMediaQuery("(max-width: 980px)");
  const allowedRoomIds = useMemo(() => {
    const binding = state?.hosted_agent_binding;
    if (!binding) return new Set<string>();
    // Associated rooms remain in the recovery model, but they are not current
    // Agent conversations and must never become dashboard message targets.
    return new Set([binding.canonical_room_id]);
  }, [state?.hosted_agent_binding]);

  const selectedRoom = useMemo(
    () =>
      state?.rooms.find(
        (room) => room.room_id === state.selected_room_id && allowedRoomIds.has(room.room_id)
      )
      ?? state?.rooms.find(
        (room) => room.room_id === state.hosted_agent_binding?.canonical_room_id
      )
      ?? null,
    [allowedRoomIds, state]
  );
  const roomTopics = useMemo(
    () =>
      (state?.topics ?? [])
        .filter((topic) => topic.room_id === selectedRoom?.room_id && !topic.archived)
        .sort((left, right) => {
          if (left.topic_id === HOME_TOPIC_ID) return -1;
          if (right.topic_id === HOME_TOPIC_ID) return 1;
          return right.updated_seq - left.updated_seq || left.title.localeCompare(right.title);
        }),
    [selectedRoom?.room_id, state?.topics]
  );
  const selectedTopic = useMemo(
    () =>
      roomTopics.find((topic) => topic.topic_id === state?.selected_topic_id)
      ?? roomTopics.find((topic) => topic.topic_id === HOME_TOPIC_ID)
      ?? roomTopics[0]
      ?? null,
    [roomTopics, state?.selected_topic_id]
  );
  const selectedChat = useMemo(
    () =>
      selectedTopic?.chats.find((chat) => chat.chat_id === state?.selected_chat_id)
      ?? selectedTopic?.chats.find((chat) => chat.active)
      ?? selectedTopic?.chats[0]
      ?? null,
    [selectedTopic, state?.selected_chat_id]
  );
  const selectedChatSelection = useMemo<ChatSelection>(
    () => ({ room: selectedRoom, topic: selectedTopic, chat: selectedChat }),
    [selectedChat, selectedRoom, selectedTopic]
  );
  const messages = useMemo(
    () =>
      (state?.messages ?? []).filter(
        (message) =>
          (!selectedRoom || message.room_id === selectedRoom.room_id)
          && (!selectedTopic || message.conversation_id === selectedTopic.topic_id)
          && (!selectedChat || message.chat_id === selectedChat.chat_id)
      ),
    [selectedChat, selectedRoom, selectedTopic, state?.messages]
  );
  const transcript = useMemo(
    () => transcriptItems(messages, state?.identity.account_id ?? null),
    [messages, state?.identity.account_id]
  );
  const liveMembers = useMemo(
    () =>
      activityLeaseIsFresh(streamConnected, activityObservedAtMs, leaseNowMs)
        ? (state?.typing_members ?? []).filter(
        (member) =>
          member.room_id === selectedRoom?.room_id
          && (!member.topic_id || member.topic_id === selectedTopic?.topic_id)
          && (!member.chat_id || member.chat_id === selectedChat?.chat_id)
        )
        : [],
    [
      activityObservedAtMs,
      leaseNowMs,
      selectedChat?.chat_id,
      selectedRoom?.room_id,
      selectedTopic?.topic_id,
      state?.typing_members,
      streamConnected,
    ]
  );
  const sites = useMemo(() => sitesFromMessages(messages), [messages]);
  const activeSite = sites.find((site) => site.id === activeSiteId) ?? sites[0] ?? null;
  const awaitingReply = pendingAgentTurns.some(
    (turn) =>
      pendingTurnLeaseIsFresh(turn, streamConnected, leaseNowMs)
      && pendingTurnMatchesSelection(turn, selectedChatSelection)
  );

  useEffect(() => {
    if (!streamConnected) {
      setActivityObservedAtMs(null);
      return;
    }
    if ((state?.typing_members.length ?? 0) === 0) {
      setActivityObservedAtMs(null);
      return;
    }
    const nowMs = Date.now();
    setActivityObservedAtMs(nowMs);
    setLeaseNowMs(nowMs);
  }, [state, streamConnected]);

  useEffect(() => {
    if (activityObservedAtMs === null && pendingAgentTurns.length === 0) return;
    const timer = window.setInterval(() => setLeaseNowMs(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [activityObservedAtMs, pendingAgentTurns.length]);

  useEffect(() => {
    if (sites.length === 0) {
      setBrowserOpen(false);
      setActiveSiteId(null);
      latestSiteIdRef.current = null;
      return;
    }
    const latestSiteId = sites[0]!.id;
    if (latestSiteIdRef.current !== latestSiteId) {
      latestSiteIdRef.current = latestSiteId;
      setActiveSiteId(latestSiteId);
    } else if (!activeSiteId || !sites.some((site) => site.id === activeSiteId)) {
      setActiveSiteId(sites[0]!.id);
    }
  }, [activeSiteId, sites]);

  useEffect(() => {
    if (!state) return;
    setPendingAgentTurns((turns) => {
      const pending = turns.filter(
        (turn) =>
          pendingTurnLeaseIsFresh(turn, streamConnected, leaseNowMs)
          && !pendingTurnIsComplete(turn, state.messages, state.identity.account_id)
      );
      return pending.length === turns.length ? turns : pending;
    });
  }, [leaseNowMs, state, streamConnected]);

  useEffect(() => {
    if (!selectedRoom) return;
    const newestRoomSeq = Math.max(
      0,
      ...(state?.messages ?? [])
        .filter((message) => message.room_id === selectedRoom.room_id)
        .map((message) => message.seq)
    );
    const markedSeq = markedReadSeqRef.current.get(selectedRoom.room_id) ?? 0;
    if (newestRoomSeq <= markedSeq) return;
    markedReadSeqRef.current.set(selectedRoom.room_id, newestRoomSeq);
    void dispatchQuiet({ MarkRoomRead: { room_id: selectedRoom.room_id } });
  }, [dispatchQuiet, selectedRoom, state?.messages]);

  const messagesFingerprint = useMemo(
    () =>
      messages
        .map((message) =>
          [
            message.message_id,
            message.kind,
            message.status,
            message.display_content,
            message.media?.length ?? 0,
          ].join(":")
        )
        .join("|"),
    [messages]
  );

  useLayoutEffect(() => {
    const node = scrollRef.current;
    if (node && shouldFollowScrollRef.current) {
      node.scrollTop = node.scrollHeight;
    }
  }, [messagesFingerprint, liveMembers.length, selectedChat?.chat_id]);

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
    () => () => {
      attachmentsRef.current.forEach(revokeAttachmentPreview);
    },
    []
  );

  useEffect(
    () => () => {
      if (typingTimerRef.current !== null) {
        window.clearTimeout(typingTimerRef.current);
      }
      if (typingRoomRef.current) {
        void dispatchQuiet({
          SetTyping: { room_id: typingRoomRef.current, is_typing: false },
        });
      }
    },
    [dispatchQuiet]
  );

  const stopTyping = useCallback(
    (roomId = typingRoomRef.current) => {
      if (typingTimerRef.current !== null) {
        window.clearTimeout(typingTimerRef.current);
        typingTimerRef.current = null;
      }
      if (roomId) {
        typingRoomRef.current = null;
        void dispatchQuiet({ SetTyping: { room_id: roomId, is_typing: false } });
      }
    },
    [dispatchQuiet]
  );

  function noteTyping(value: string) {
    if (!selectedRoom || selectedRoom.state !== "Connected") return;
    if (!value.trim()) {
      stopTyping(selectedRoom.room_id);
      return;
    }
    if (typingRoomRef.current !== selectedRoom.room_id) {
      typingRoomRef.current = selectedRoom.room_id;
      void dispatchQuiet({ SetTyping: { room_id: selectedRoom.room_id, is_typing: true } });
    }
    if (typingTimerRef.current !== null) window.clearTimeout(typingTimerRef.current);
    typingTimerRef.current = window.setTimeout(
      () => stopTyping(selectedRoom.room_id),
      TYPING_IDLE_MS
    );
  }

  async function send(event: FormEvent) {
    event.preventDefault();
    const text = draft.trim();
    if ((!text && attachments.length === 0) || !selectedRoom || sending) return;
    if (attachments.length > 0 && selectedTopic && !selectedChat) {
      setActionError("Start a chat in this topic before attaching files.");
      return;
    }
    setSending(true);
    setActionError(null);
    stopTyping(selectedRoom.room_id);
    const pendingTurnStartedAtMs = Date.now();
    const pendingTurn = beginPendingChatTurn(
      selectedChatSelection,
      messages,
      pendingTurnStartedAtMs
    );
    if (pendingTurn) {
      setLeaseNowMs(pendingTurnStartedAtMs);
      setPendingAgentTurns((turns) => [
        ...turns.filter((turn) => !pendingTurnMatchesSelection(turn, selectedChatSelection)),
        pendingTurn,
      ]);
    }
    try {
      let next: HostedChatState;
      if (attachments.length > 0) {
        const formData = new FormData();
        formData.set("room_id", selectedRoom.room_id);
        if (selectedTopic) formData.set("topic_id", selectedTopic.topic_id);
        if (selectedChat) formData.set("chat_id", selectedChat.chat_id);
        formData.set("caption", text);
        for (const attachment of attachments) formData.append("files", attachment.file);
        next = await uploadAttachments(formData);
        const uploadError = attachmentSendError(next);
        if (uploadError) throw new Error(uploadError);
      } else {
        next = await dispatch(messageAction(selectedRoom.room_id, text, selectedTopic, selectedChat));
      }
      setDraft("");
      setAttachments((current) => {
        current.forEach(revokeAttachmentPreview);
        return [];
      });
      requestAnimationFrame(() => textareaRef.current?.focus());
    } catch (caught) {
      if (pendingTurn) {
        setPendingAgentTurns((turns) => turns.filter((turn) => turn !== pendingTurn));
      }
      setActionError(hostedChatErrorMessage(caught));
    } finally {
      setSending(false);
    }
  }

  function addFiles(files: FileList | File[]) {
    const incoming = Array.from(files);
    const accepted = incoming.filter((file) => file.size <= MAX_ATTACHMENT_BYTES);
    const oversized = incoming.find((file) => file.size > MAX_ATTACHMENT_BYTES);
    if (oversized) {
      setActionError(`${oversized.name} is larger than ${formatBytes(MAX_ATTACHMENT_BYTES)}.`);
    }
    setAttachments((current) => {
      const remaining = Math.max(0, MAX_ATTACHMENTS - current.length);
      const availableBytes = Math.max(
        0,
        MAX_ATTACHMENT_TOTAL_BYTES
          - current.reduce((total, attachment) => total + attachment.file.size, 0)
      );
      let selectedBytes = 0;
      const selected = accepted.slice(0, remaining).filter((file) => {
        if (selectedBytes + file.size > availableBytes) return false;
        selectedBytes += file.size;
        return true;
      });
      if (incoming.length > remaining) {
        setActionError(`You can attach up to ${MAX_ATTACHMENTS} files at a time.`);
      } else if (selected.length < accepted.slice(0, remaining).length) {
        setActionError(`Attachments can total up to ${formatBytes(MAX_ATTACHMENT_TOTAL_BYTES)}.`);
      }
      return [
        ...current,
        ...selected.map((file) => ({
          id: attachmentId(file),
          file,
          previewUrl: file.type.startsWith("image/") ? URL.createObjectURL(file) : null,
        })),
      ];
    });
  }

  function removeAttachment(id: string) {
    setAttachments((current) => {
      const removed = current.find((attachment) => attachment.id === id);
      if (removed) revokeAttachmentPreview(removed);
      return current.filter((attachment) => attachment.id !== id);
    });
  }

  async function renameChat(event: FormEvent) {
    event.preventDefault();
    if (!selectedRoom || !selectedTopic || !selectedChat || !renameTitle.trim()) return;
    try {
      await dispatch({
        RenameChat: {
          room_id: selectedRoom.room_id,
          topic_id: selectedTopic.topic_id,
          chat_id: selectedChat.chat_id,
          title: renameTitle.trim(),
        },
      });
      setRenameOpen(false);
    } catch (caught) {
      setActionError(hostedChatErrorMessage(caught));
    }
  }

  const connected = ownerClaimed && selectedRoom?.state === "Connected";
  const activityLabel = sharedLiveActivityLabel(liveMembers, machineLabel, awaitingReply);

  return (
    <div className="finite-chat finite-chat--embedded">
      <div className="finite-chat__workspace">
        <header className="finite-chat__topbar">
          <div className="finite-chat__breadcrumb">
            <button
              type="button"
              className="ocean-icon-button finite-chat__sidebar-toggle"
              aria-label="Open chats"
              onClick={() => window.dispatchEvent(new Event("finite:open-agent-sidebar"))}
            >
              <PanelLeftIcon className="size-4" />
            </button>
            <span>{selectedTopic?.title ?? "Home"}</span>
            <ChevronRightIcon className="size-4" />
            <strong>{selectedChat?.title ?? machineLabel}</strong>
            {selectedChat ? (
              <button
                type="button"
                className="ocean-icon-button finite-chat__rename-button"
                aria-label="Rename chat"
                onClick={() => {
                  setRenameTitle(selectedChat.title);
                  setRenameOpen(true);
                }}
              >
                <PencilIcon className="size-3.5" />
              </button>
            ) : null}
          </div>

          <div className="finite-chat__topbar-actions">
            {!streamConnected && state ? (
              <span className="finite-chat__relay-warning">Reconnecting</span>
            ) : null}
            {sites.length > 0 ? (
              <button
                type="button"
                className="ocean-pill-button"
                aria-expanded={browserOpen}
                onClick={() => setBrowserOpen((value) => !value)}
              >
                <MonitorIcon className="size-4" />
                <span>Preview</span>
              </button>
            ) : null}
          </div>
        </header>

        <Group
          orientation="horizontal"
          className={browserOpen && activeSite ? "finite-chat__split has-browser" : "finite-chat__split"}
        >
          <Panel className="finite-chat__main-panel" defaultSize={browserOpen ? "54%" : "100%"} minSize="34%">
            <section className="finite-chat__main" aria-label="Web chat">
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
                {!state && !transportError ? <ChatLoading label="Opening your chat…" /> : null}
                {state && !selectedRoom ? (
                  <EmptyChat title="Connecting to your agent" body="Your chat is getting ready." />
                ) : null}
                {selectedRoom && messages.length === 0 ? (
                  <EmptyChat title="What should we work on?" body="Start here, or make a new chat inside this topic." />
                ) : null}
                {messages.length > 0 ? (
                  <div className="finite-chat__messages" aria-live="polite">
                    {selectedRoom?.can_load_older && messages[0] ? (
                      <button
                        type="button"
                        className="finite-chat__load-older-button"
                        onClick={() =>
                          void dispatch({
                            LoadOlderMessages: {
                              room_id: selectedRoom.room_id,
                              before_message_id: messages[0]!.message_id,
                              limit: 80,
                            },
                          })
                        }
                      >
                        Load earlier messages
                      </button>
                    ) : null}
                    {transcript.map((item) =>
                      item.type === "message" ? (
                        <MessageRow
                          key={item.message.message_id}
                          apiBase={apiBase}
                          message={item.message}
                          ownAccountId={state?.identity.account_id ?? ""}
                        />
                      ) : (
                        <ToolRollup key={item.id} messages={item.messages} />
                      )
                    )}
                    {activityLabel ? <LiveActivity label={activityLabel} /> : null}
                  </div>
                ) : null}
              </div>

              {showLatest ? (
                <button
                  type="button"
                  className="finite-chat__scroll-bottom-button"
                  onClick={() => {
                    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
                    shouldFollowScrollRef.current = true;
                    setShowLatest(false);
                  }}
                >
                  <ArrowDownIcon className="size-4" />
                  <span>Latest</span>
                </button>
              ) : null}

              {transportError || claimError || actionError ? (
                <div className="finite-chat__send-error" role="alert">
                  <strong>Chat needs attention</strong>
                  <span>{transportError ?? claimError ?? actionError}</span>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      if (transportError) {
                        void (bindingRecoveryRequired ? recoverBinding() : load(true));
                      } else if (claimError) {
                        void claimOwner();
                      } else {
                        setActionError(null);
                      }
                    }}
                  >
                    {transportError || claimError ? <RotateCcwIcon /> : null}
                    {transportError
                      ? bindingRecoveryRequired
                        ? "Finish chat setup"
                        : "Retry load"
                      : claimError
                        ? "Retry claim"
                        : "Dismiss"}
                  </Button>
                </div>
              ) : null}

              <div className="finite-chat__composer-wrap">
                <form
                  className={`finite-chat__composer ${isDragOver ? "is-drag-over" : ""}`}
                  onSubmit={send}
                  onDragOver={(event) => {
                    event.preventDefault();
                    setIsDragOver(true);
                  }}
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
                          {attachment.previewUrl ? (
                            // eslint-disable-next-line @next/next/no-img-element
                            <img src={attachment.previewUrl} alt="" />
                          ) : (
                            <FileTextIcon className="size-4" />
                          )}
                          <span>{attachment.file.name}</span>
                          <button
                            type="button"
                            aria-label={`Remove ${attachment.file.name}`}
                            onClick={() => removeAttachment(attachment.id)}
                          >
                            <XIcon className="size-3.5" />
                          </button>
                        </div>
                      ))}
                    </div>
                  ) : null}
                  <textarea
                    ref={textareaRef}
                    aria-label="Message your agent"
                    placeholder={connected ? `Ask ${machineLabel} anything` : CHAT_WAITING_FOR_AGENT_MESSAGE}
                    value={draft}
                    disabled={!connected || sending}
                    rows={1}
                    onBlur={() => stopTyping(selectedRoom?.room_id)}
                    onChange={(event) => {
                      setDraft(event.target.value);
                      noteTyping(event.target.value);
                    }}
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
                    <div className="finite-chat__composer-left">
                      <input
                        ref={fileInputRef}
                        type="file"
                        hidden
                        multiple
                        onChange={(event) => {
                          if (event.currentTarget.files) addFiles(event.currentTarget.files);
                          event.currentTarget.value = "";
                        }}
                      />
                      <button
                        type="button"
                        className="finite-chat__tool-button"
                        disabled={!connected || sending}
                        aria-label="Attach files"
                        onClick={() => fileInputRef.current?.click()}
                      >
                        <PaperclipIcon className="size-4" />
                      </button>
                    </div>
                    <button
                      type="submit"
                      className="finite-chat__send-button"
                      aria-label="Send message"
                      disabled={!connected || (!draft.trim() && attachments.length === 0) || sending}
                    >
                      {sending ? <Loader2Icon className="finite-chat__spin" /> : <ArrowUpIcon />}
                    </button>
                  </div>
                </form>
              </div>
            </section>
          </Panel>

          {browserOpen && activeSite ? (
            <>
              <Separator className="finite-chat__preview-resizer" />
              <Panel className="finite-chat__desktop-preview-panel" defaultSize="46%" minSize="28%" maxSize="70%">
                <BrowserPanel
                  activeSite={activeSite}
                  className="finite-chat__preview finite-chat__preview--desktop"
                  machineId={machineId}
                  onClose={() => setBrowserOpen(false)}
                  onSelectSite={setActiveSiteId}
                  sites={sites}
                />
              </Panel>
            </>
          ) : null}
        </Group>
      </div>

      {activeSite && mobilePreview ? (
        <Drawer.Root open={browserOpen} onOpenChange={setBrowserOpen} direction="bottom" handleOnly>
          <Drawer.Portal>
            <Drawer.Overlay className="finite-chat__preview-backdrop" />
            <Drawer.Content className="finite-chat__preview-sheet-panel" aria-describedby={undefined}>
              <Drawer.Handle className="finite-chat__sheet-handle" />
              <Drawer.Title className="finite-chat__sheet-title">Site preview</Drawer.Title>
              <BrowserPanel
                activeSite={activeSite}
                className="finite-chat__preview finite-chat__preview--sheet"
                machineId={machineId}
                onClose={() => setBrowserOpen(false)}
                onSelectSite={setActiveSiteId}
                sites={sites}
              />
            </Drawer.Content>
          </Drawer.Portal>
        </Drawer.Root>
      ) : null}

      <Dialog open={renameOpen} onOpenChange={setRenameOpen}>
        <DialogContent>
          <form className="finite-chat__rename-form" onSubmit={renameChat}>
            <DialogHeader>
              <DialogTitle>Rename chat</DialogTitle>
              <DialogDescription>Choose a name that makes this chat easy to find later.</DialogDescription>
            </DialogHeader>
            <div className="finite-chat__rename-field">
              <label htmlFor="finite-chat-rename-title">Name</label>
              <Input
                id="finite-chat-rename-title"
                autoFocus
                maxLength={120}
                value={renameTitle}
                onChange={(event) => setRenameTitle(event.target.value)}
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setRenameOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={!renameTitle.trim()}>Save</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

    </div>
  );
}

function LiveActivity({ label }: { label: string }) {
  return (
    <div className="finite-chat__live-activity" aria-live="polite">
      <span className="finite-chat__live-dots" aria-hidden><i /><i /><i /></span>
      <span>{label}</span>
    </div>
  );
}

function ToolRollup({ messages }: { messages: HostedChatMessage[] }) {
  const running = messages.some((message) => message.status === "running");
  const steps = messages.flatMap((message) => messageContent(message).split(/\n+/u).filter(Boolean));
  const label = running
    ? steps.length > 0 ? `Working · ${steps.length} ${pluralize("step", steps.length)}` : "Working"
    : `Worked through ${steps.length || messages.length} ${pluralize("step", steps.length || messages.length)}`;
  return (
    <details className="finite-chat__tool-rollup" open={running || undefined}>
      <summary>
        {running ? <Loader2Icon className="size-4 finite-chat__spin" /> : <WrenchIcon className="size-4" />}
        <span>{label}</span>
        <ChevronRightIcon className="size-4" />
      </summary>
      <div className="finite-chat__tool-rollup-body">
        {messages.map((message) => <pre key={message.message_id}>{messageContent(message) || "Done"}</pre>)}
      </div>
    </details>
  );
}

function MessageRow({
  apiBase,
  message,
  ownAccountId,
}: {
  apiBase: string;
  message: HostedChatMessage;
  ownAccountId: string;
}) {
  const content = messageContent(message);
  if (message.sender_account_id === ownAccountId || (!ownAccountId && message.is_mine)) {
    return (
      <article className="finite-chat__message finite-chat__message--user">
        <div>
          <MessageAttachments apiBase={apiBase} message={message} compact />
          {content ? <p>{content}</p> : null}
          <time className="finite-chat__message-time">{deliveryText(message) || message.display_timestamp}</time>
        </div>
      </article>
    );
  }
  return (
    <article className="finite-chat__message finite-chat__message--agent">
      <MessageAttachments apiBase={apiBase} message={message} />
      {content ? <MarkdownMessage text={content} /> : null}
      <time className="finite-chat__message-time">{message.display_timestamp}</time>
    </article>
  );
}

function MessageAttachments({ apiBase, compact = false, message }: { apiBase: string; compact?: boolean; message: HostedChatMessage }) {
  const media = message.media ?? [];
  if (media.length === 0) return null;
  return (
    <div className={compact ? "finite-chat__media-grid is-compact" : "finite-chat__media-grid"}>
      {media.map((attachment) => (
        <AttachmentCard key={attachment.attachment_id} apiBase={apiBase} attachment={attachment} message={message} compact={compact} />
      ))}
    </div>
  );
}

function AttachmentCard({ apiBase, attachment, compact, message }: { apiBase: string; attachment: HostedChatMediaAttachment; compact: boolean; message: HostedChatMessage }) {
  const href = `${apiBase}/attachments/${encodeURIComponent(message.room_id)}/${encodeURIComponent(message.message_id)}/${encodeURIComponent(attachment.attachment_id)}`;
  const cardClassName = compact
    ? "finite-chat__image-card is-compact"
    : "finite-chat__image-card";
  if (attachment.kind === "Video" || attachment.mime_type.startsWith("video/")) {
    return (
      <span className={`${cardClassName} finite-chat__playable-card`}>
        <video src={href} controls preload="metadata" aria-label={attachment.filename} />
        <AttachmentCaption href={href} name={attachment.filename} />
      </span>
    );
  }
  if (attachment.kind === "VoiceNote" || attachment.mime_type.startsWith("audio/")) {
    return (
      <span className={`${cardClassName} finite-chat__playable-card is-audio`}>
        <audio src={href} controls preload="metadata" aria-label={attachment.filename} />
        <AttachmentCaption href={href} name={attachment.filename} />
      </span>
    );
  }
  if (attachment.kind !== "Image") {
    return <a href={href} className="finite-chat__file-card"><FileTextIcon className="size-4" /><span>{attachment.filename}</span></a>;
  }
  return (
    <span className={cardClassName}>
      <a className="finite-chat__image-link" href={href} target="_blank" rel="noreferrer">
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img src={href} alt={attachment.filename} />
      </a>
      <AttachmentCaption href={href} name={attachment.filename} />
    </span>
  );
}

function AttachmentCaption({ href, name }: { href: string; name: string }) {
  return (
    <span className="finite-chat__image-caption">
      <span>{name}</span>
      <span className="finite-chat__image-actions">
        <a href={href} download={name} aria-label={`Download ${name}`}><DownloadIcon className="size-3.5" /></a>
        <ShareAttachmentButton href={href} name={name} />
      </span>
    </span>
  );
}

function ShareAttachmentButton({ href, name }: { href: string; name: string }) {
  if (typeof navigator === "undefined" || !("share" in navigator)) return null;
  return <button type="button" aria-label={`Share ${name}`} onClick={() => void navigator.share({ title: name, url: new URL(href, window.location.href).toString() }).catch(() => undefined)}><Share2Icon className="size-3.5" /></button>;
}

function MarkdownMessage({ text }: { text: string }) {
  return (
    <div className="finite-chat__assistant-text finite-chat__markdown">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={{ a: MarkdownLink, table: MarkdownTable }}>{text}</ReactMarkdown>
    </div>
  );
}

function MarkdownTable({ children, ...props }: ComponentProps<"table">) {
  return <div className="finite-chat__table-scroll"><table {...props}>{children}</table></div>;
}

function MarkdownLink({ children, href }: ComponentProps<"a">) {
  return <a href={typeof href === "string" ? href : ""} target="_blank" rel="noreferrer">{children}</a>;
}

function EmptyChat({ body, title }: { body: string; title: string }) {
  return (
    <div className="finite-chat__empty finite-chat__empty--solo">
      <span className="finite-chat__empty-logo" aria-hidden />
      <h1 className="finite-chat__empty-title">{title}</h1>
      <p>{body}</p>
    </div>
  );
}

function ChatLoading({ label }: { label: string }) {
  return <div className="finite-chat__notice"><Loader2Icon className="finite-chat__spin" /><span>{label}</span></div>;
}

function BrowserPanel({ activeSite, className, machineId, onClose, onSelectSite, sites }: { activeSite: PreviewSite; className: string; machineId: string; onClose: () => void; onSelectSite: (id: string) => void; sites: PreviewSite[] }) {
  const [frameState, setFrameState] = useState<{
    requestKey: string;
    url: string | null;
    error: boolean;
  }>({ requestKey: "", url: null, error: false });
  const [reloadVersion, setReloadVersion] = useState(0);
  const requestKey = `${activeSite.id}:${reloadVersion}`;
  const frameUrl = frameState.requestKey === requestKey ? frameState.url : null;
  const frameError = frameState.requestKey === requestKey && frameState.error;

  useEffect(() => {
    let disposed = false;
    const currentRequestKey = `${activeSite.id}:${reloadVersion}`;
    fetch(`/api/site-previews/machines/${encodeURIComponent(machineId)}/session`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ url: activeSite.url }),
    })
      .then(async (response) => {
        if (!response.ok) throw new Error("preview session unavailable");
        return response.json() as Promise<{ url?: unknown }>;
      })
      .then((payload) => {
        if (!disposed) {
          setFrameState({
            requestKey: currentRequestKey,
            url: typeof payload.url === "string" ? payload.url : null,
            error: typeof payload.url !== "string",
          });
        }
      })
      .catch(() => {
        if (!disposed) {
          setFrameState({ requestKey: currentRequestKey, url: null, error: true });
        }
      });
    return () => {
      disposed = true;
    };
  }, [activeSite.id, activeSite.url, machineId, reloadVersion]);

  return (
    <aside className={className} aria-label="Site preview">
      <div className="finite-chat__browser">
        <div className="finite-chat__browser-chrome">
          <span className="finite-chat__traffic-lights" aria-hidden><span /><span /><span /></span>
          {sites.length > 1 ? (
            <select className="finite-chat__site-native-select" aria-label="Select site preview" value={activeSite.id} onChange={(event) => onSelectSite(event.target.value)}>
              {sites.map((site) => <option key={site.id} value={site.id}>{site.label}</option>)}
            </select>
          ) : <span className="finite-chat__site-switcher finite-chat__site-switcher--single">{activeSite.label}</span>}
          <input aria-label="Preview URL" readOnly value={activeSite.url} />
          <div className="finite-chat__browser-actions">
            <button type="button" aria-label="Copy preview link" onClick={() => void navigator.clipboard.writeText(activeSite.url)}><CopyIcon className="size-3.5" /></button>
            <a href={activeSite.url} target="_blank" rel="noreferrer" aria-label="Open preview"><ExternalLinkIcon className="size-3.5" /></a>
            <button type="button" aria-label="Reload preview" onClick={() => setReloadVersion((value) => value + 1)}><RefreshCwIcon className="size-3.5" /></button>
            <button type="button" aria-label="Close preview" onClick={onClose}><XIcon className="size-3.5" /></button>
          </div>
        </div>
        <div className="finite-chat__browser-viewport">
          {frameUrl ? (
            <iframe key={`${activeSite.id}:${reloadVersion}`} className="finite-chat__browser-iframe" src={frameUrl} title={activeSite.label} sandbox="allow-forms allow-same-origin allow-scripts" referrerPolicy="no-referrer" />
          ) : frameError ? (
            <div className="finite-chat__notice">Preview isn&apos;t available right now.</div>
          ) : (
            <ChatLoading label="Opening preview…" />
          )}
        </div>
      </div>
    </aside>
  );
}

function messageAction(roomId: string, text: string, topic: HostedChatTopic | null, chat: HostedChatSummary | null): HostedChatAction {
  if (topic && chat) return { SendChatMessage: { room_id: roomId, topic_id: topic.topic_id, chat_id: chat.chat_id, text } };
  if (topic) return { SendTopicMessage: { room_id: roomId, topic_id: topic.topic_id, text } };
  return { SendMessage: { room_id: roomId, text } };
}

function sitesFromMessages(messages: HostedChatMessage[]) {
  const seen = new Set<string>();
  const sites: PreviewSite[] = [];
  const urlPattern = /https?:\/\/[^\s<>()\[\]{}"']+/giu;
  for (const message of [...messages].reverse()) {
    for (const raw of messageContent(message).match(urlPattern) ?? []) {
      const value = raw.replace(/[.,;:!?]+$/u, "");
      try {
        const url = new URL(value);
        const local = url.hostname.endsWith(".localhost");
        const finite = url.hostname.endsWith(".finite.chat");
        const reservedHost = /^(?:api|git)\./u.test(url.hostname);
        const repository = url.pathname.endsWith(".git");
        if ((!local && !finite) || reservedHost || repository || seen.has(url.toString())) continue;
        seen.add(url.toString());
        sites.push({ id: url.toString(), label: url.hostname, url: url.toString() });
      } catch {
        // Ignore malformed prose URLs.
      }
    }
  }
  return sites.slice(0, 8);
}

function deliveryText(message: HostedChatMessage) {
  const delivery = message.outbound_delivery;
  if (!delivery) return null;
  if (typeof delivery.server_delivery === "object" && "Failed" in delivery.server_delivery) return "Not delivered";
  if (delivery.server_delivery === "Undelivered") return "Sending…";
  return "Delivered";
}

function useMediaQuery(query: string) {
  const [matches, setMatches] = useState(false);
  useEffect(() => {
    const media = window.matchMedia(query);
    const update = () => setMatches(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, [query]);
  return matches;
}

function attachmentId(file: File) {
  const random = typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : Math.random().toString(36).slice(2);
  return `${file.name}-${file.size}-${file.lastModified}-${random}`;
}

function revokeAttachmentPreview(attachment: PendingAttachment) {
  if (attachment.previewUrl) URL.revokeObjectURL(attachment.previewUrl);
}

function pluralize(word: string, count: number) {
  return count === 1 ? word : `${word}s`;
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
