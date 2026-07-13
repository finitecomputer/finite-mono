"use client";

import Link from "next/link";
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
  HashIcon,
  Loader2Icon,
  LogOutIcon,
  MonitorIcon,
  MonitorSmartphoneIcon,
  MoreHorizontalIcon,
  PanelLeftIcon,
  PaperclipIcon,
  PencilIcon,
  PlugIcon,
  PlusIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  Share2Icon,
  WrenchIcon,
  XIcon,
  type LucideIcon,
} from "lucide-react";

import { FiniteBrand } from "@/components/finite-brand";
import {
  CHAT_INVALID_UPDATE_MESSAGE,
  CHAT_TOPIC_DESCRIPTION,
  CHAT_UNAVAILABLE_MESSAGE,
  CHAT_WAITING_FOR_AGENT_MESSAGE,
} from "@/lib/chat-product-copy";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import type {
  HostedChatAction,
  HostedChatDevice,
  HostedChatMediaAttachment,
  HostedChatMessage,
  HostedChatState,
  HostedChatSummary,
  HostedChatTopic,
  HostedChatTypingMember,
} from "@/lib/hosted-web-device";
import {
  initialHostedChatSnapshotSource,
  nextHostedChatSnapshotGeneration,
  recordHostedChatSnapshot,
  shouldApplyHttpHostedChatSnapshot,
  shouldApplyStreamHostedChatSnapshot,
} from "@/lib/hosted-web-chat-snapshots";
import {
  runInitialHostedChatRetries,
  shouldRetryHostedChatRequest,
  type HostedChatRetryAttempt,
} from "@/lib/hosted-web-chat-retry";
import {
  beginPendingChatTurn,
  attachmentSendError,
  liveActivityLabel as sharedLiveActivityLabel,
  messageContent,
  pendingTurnIsComplete,
  pendingTurnMatchesSelection,
  transcriptItems,
  type ChatSelection,
  type PendingChatTurn,
} from "@finite/chat-ui";

const STREAM_RECONNECT_DELAY_MS = 1_000;
const TYPING_IDLE_MS = 2_200;
const MAX_ATTACHMENT_BYTES = 32 * 1024 * 1024;
const MAX_ATTACHMENT_TOTAL_BYTES = 64 * 1024 * 1024;
const MAX_ATTACHMENTS = 8;
const AUTO_FOLLOW_SCROLL_THRESHOLD_PX = 120;
const HOME_TOPIC_ID = "home";

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
  connectionsHref,
  initialDraft,
  machineId,
  machineLabel,
  showSkills,
  viewerEmail,
}: {
  connectionsHref?: string | null;
  initialDraft?: string;
  machineId: string;
  machineLabel: string;
  showSkills: boolean;
  viewerEmail?: string | null;
}) {
  const apiBase = `/api/chat/machines/${encodeURIComponent(machineId)}/hosted-device`;
  const [state, setState] = useState<HostedChatState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [draft, setDraft] = useState(initialDraft ?? "");
  const [attachments, setAttachments] = useState<PendingAttachment[]>([]);
  const [streamConnected, setStreamConnected] = useState(false);
  const [ownerClaimed, setOwnerClaimed] = useState(false);
  const [pendingAgentTurns, setPendingAgentTurns] = useState<PendingChatTurn[]>([]);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [isDragOver, setIsDragOver] = useState(false);
  const [createTopicOpen, setCreateTopicOpen] = useState(false);
  const [createTopicTitle, setCreateTopicTitle] = useState("");
  const [renameOpen, setRenameOpen] = useState(false);
  const [renameTitle, setRenameTitle] = useState("");
  const [devicesOpen, setDevicesOpen] = useState(false);
  const [deviceBusy, setDeviceBusy] = useState(false);
  const [deviceToRevoke, setDeviceToRevoke] = useState<HostedChatDevice | null>(null);
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
  const snapshotSourceRef = useRef(initialHostedChatSnapshotSource());
  const stateLoadRef = useRef<Promise<HostedChatRetryAttempt> | null>(null);
  const ownerClaimRef = useRef<Promise<HostedChatRetryAttempt> | null>(null);
  const mobilePreview = useMediaQuery("(max-width: 980px)");
  const hasState = state !== null;

  const applyHttpSnapshot = useCallback((next: HostedChatState, requestGeneration: number) => {
    const source = snapshotSourceRef.current;
    if (!shouldApplyHttpHostedChatSnapshot(source, requestGeneration, next.rev)) {
      return false;
    }
    snapshotSourceRef.current = recordHostedChatSnapshot(source, next.rev, false);
    setState((current) => ({
      ...next,
      hosted_agent_binding: next.hosted_agent_binding ?? current?.hosted_agent_binding ?? null,
    }));
    return true;
  }, []);

  const load = useCallback(() => {
    if (stateLoadRef.current) return stateLoadRef.current;
    const requestGeneration = snapshotSourceRef.current.generation;
    const pending = (async (): Promise<HostedChatRetryAttempt> => {
      try {
        const next = await chatRequest<HostedChatState>(`${apiBase}/state`);
        applyHttpSnapshot(next, requestGeneration);
        setError(null);
        return "succeeded";
      } catch (caught) {
        setError(errorMessage(caught));
        const status = caught instanceof HostedChatHttpError ? caught.status : null;
        return shouldRetryHostedChatRequest(status) ? "retry" : "stop";
      }
    })();
    stateLoadRef.current = pending;
    void pending.finally(() => {
      if (stateLoadRef.current === pending) stateLoadRef.current = null;
    });
    return pending;
  }, [apiBase, applyHttpSnapshot]);

  const claimOwner = useCallback(() => {
    if (ownerClaimRef.current) return ownerClaimRef.current;
    const pending = (async (): Promise<HostedChatRetryAttempt> => {
      try {
        await chatRequest<{ claimed: true }>(`${apiBase}/claim`, { method: "POST" });
        setOwnerClaimed(true);
        setError(null);
        return "succeeded";
      } catch (caught) {
        setError(errorMessage(caught));
        const status = caught instanceof HostedChatHttpError ? caught.status : null;
        return shouldRetryHostedChatRequest(status) ? "retry" : "stop";
      }
    })();
    ownerClaimRef.current = pending;
    void pending.finally(() => {
      if (ownerClaimRef.current === pending) ownerClaimRef.current = null;
    });
    return pending;
  }, [apiBase]);

  const dispatch = useCallback(
    async (action: HostedChatAction) => {
      const requestGeneration = snapshotSourceRef.current.generation;
      const next = await chatRequest<HostedChatState>(`${apiBase}/actions`, {
        method: "POST",
        body: JSON.stringify(action),
      });
      applyHttpSnapshot(next, requestGeneration);
      return next;
    },
    [apiBase, applyHttpSnapshot]
  );

  const dispatchQuiet = useCallback(
    async (action: HostedChatAction) => {
      try {
        return await dispatch(action);
      } catch {
        return null;
      }
    },
    [dispatch]
  );

  useEffect(() => {
    if (hasState) return;
    const controller = new AbortController();
    void runInitialHostedChatRetries(load, controller.signal);
    return () => controller.abort();
  }, [hasState, load]);

  useEffect(() => {
    if (!hasState || ownerClaimed) return;
    const controller = new AbortController();
    void runInitialHostedChatRetries(claimOwner, controller.signal);
    return () => controller.abort();
  }, [claimOwner, hasState, ownerClaimed]);

  useEffect(() => {
    if (!hasState) {
      return;
    }
    let disposed = false;
    let events: EventSource | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    const connect = () => {
      if (disposed) {
        return;
      }
      const nextEvents = new EventSource(`${apiBase}/updates`);
      events = nextEvents;
      snapshotSourceRef.current = nextHostedChatSnapshotGeneration(snapshotSourceRef.current);
      const onState = (event: MessageEvent<string>) => {
        try {
          const next = JSON.parse(event.data) as HostedChatState;
          if (events !== nextEvents) {
            return;
          }
          const source = snapshotSourceRef.current;
          if (!shouldApplyStreamHostedChatSnapshot(source, next.rev)) {
            return;
          }
          snapshotSourceRef.current = recordHostedChatSnapshot(source, next.rev, true);
          setState((current) => ({
            ...next,
            hosted_agent_binding: next.hosted_agent_binding ?? current?.hosted_agent_binding ?? null,
          }));
          setError(null);
          setStreamConnected(true);
        } catch {
          setError(CHAT_INVALID_UPDATE_MESSAGE);
        }
      };
      nextEvents.addEventListener("open", () => setStreamConnected(true));
      nextEvents.addEventListener("state", onState as EventListener);
      nextEvents.addEventListener("error", () => {
        if (disposed || events !== nextEvents) {
          return;
        }
        nextEvents.close();
        events = null;
        setStreamConnected(false);
        setError((current) => current ?? "Reconnecting…");
        reconnectTimer = setTimeout(connect, STREAM_RECONNECT_DELAY_MS);
      });
    };

    connect();
    return () => {
      disposed = true;
      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
      }
      events?.close();
    };
  }, [apiBase, hasState]);

  const selectedRoom = useMemo(
    () =>
      state?.rooms.find((room) => room.room_id === state.selected_room_id)
      ?? state?.rooms.find((room) => room.is_agent_chat)
      ?? state?.rooms[0]
      ?? null,
    [state]
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
  const canonicalRoomId = state?.hosted_agent_binding?.canonical_room_id ?? selectedRoom?.room_id;
  const canonicalTopics = useMemo(
    () => (state?.topics ?? []).filter(
      (topic) => topic.room_id === canonicalRoomId && !topic.archived
    ),
    [canonicalRoomId, state?.topics]
  );
  const previousTopics = useMemo(() => {
    const associated = new Set(state?.hosted_agent_binding?.associated_room_ids ?? []);
    return (state?.topics ?? []).filter(
      (topic) =>
        topic.room_id !== canonicalRoomId
        && associated.has(topic.room_id)
        && !topic.archived
    );
  }, [canonicalRoomId, state?.hosted_agent_binding?.associated_room_ids, state?.topics]);
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
      (state?.typing_members ?? []).filter(
        (member) =>
          member.room_id === selectedRoom?.room_id
          && (!member.topic_id || member.topic_id === selectedTopic?.topic_id)
          && (!member.chat_id || member.chat_id === selectedChat?.chat_id)
      ),
    [selectedChat?.chat_id, selectedRoom?.room_id, selectedTopic?.topic_id, state?.typing_members]
  );
  const sites = useMemo(() => sitesFromMessages(messages), [messages]);
  const activeSite = sites.find((site) => site.id === activeSiteId) ?? sites[0] ?? null;
  const awaitingReply = pendingAgentTurns.some((turn) =>
    pendingTurnMatchesSelection(turn, selectedChatSelection)
  );

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
        (turn) => !pendingTurnIsComplete(turn, state.messages, state.identity.account_id)
      );
      return pending.length === turns.length ? turns : pending;
    });
  }, [state]);

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
      setError("Start a chat in this topic before attaching files.");
      return;
    }
    setSending(true);
    setError(null);
    stopTyping(selectedRoom.room_id);
    const pendingTurn = beginPendingChatTurn(selectedChatSelection, messages);
    if (pendingTurn) {
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
        const requestGeneration = snapshotSourceRef.current.generation;
        next = await chatRequest<HostedChatState>(`${apiBase}/attachments`, {
          method: "POST",
          body: formData,
        });
        applyHttpSnapshot(next, requestGeneration);
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
      setError(errorMessage(caught));
    } finally {
      setSending(false);
    }
  }

  function addFiles(files: FileList | File[]) {
    const incoming = Array.from(files);
    const accepted = incoming.filter((file) => file.size <= MAX_ATTACHMENT_BYTES);
    const oversized = incoming.find((file) => file.size > MAX_ATTACHMENT_BYTES);
    if (oversized) {
      setError(`${oversized.name} is larger than ${formatBytes(MAX_ATTACHMENT_BYTES)}.`);
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
        setError(`You can attach up to ${MAX_ATTACHMENTS} files at a time.`);
      } else if (selected.length < accepted.slice(0, remaining).length) {
        setError(`Attachments can total up to ${formatBytes(MAX_ATTACHMENT_TOTAL_BYTES)}.`);
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

  async function openTopic(topic: HostedChatTopic) {
    setError(null);
    try {
      await dispatch({ OpenTopic: { room_id: topic.room_id, topic_id: topic.topic_id } });
      setSidebarOpen(false);
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }

  async function openChat(topic: HostedChatTopic, chat: HostedChatSummary) {
    setError(null);
    try {
      await dispatch({
        OpenChat: { room_id: topic.room_id, topic_id: topic.topic_id, chat_id: chat.chat_id },
      });
      setSidebarOpen(false);
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }

  async function createChat(topic = canonicalTopics.find((item) => item.topic_id === HOME_TOPIC_ID) ?? canonicalTopics[0]) {
    if (!canonicalRoomId || !topic) return;
    setError(null);
    try {
      await dispatch({
        StartTopicChatIntent: {
          room_id: canonicalRoomId,
          topic_id: topic.topic_id,
          reason: null,
          intent_key: crypto.randomUUID(),
        },
      });
      setSidebarOpen(false);
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }

  async function createTopic(event: FormEvent) {
    event.preventDefault();
    if (!canonicalRoomId || !createTopicTitle.trim()) return;
    try {
      await dispatch({
        CreateTopic: { room_id: canonicalRoomId, title: createTopicTitle.trim() },
      });
      setCreateTopicTitle("");
      setCreateTopicOpen(false);
      setSidebarOpen(false);
    } catch (caught) {
      setError(errorMessage(caught));
    }
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
      setError(errorMessage(caught));
    }
  }

  async function openDevices() {
    setDevicesOpen(true);
    setDeviceToRevoke(null);
    setDeviceBusy(true);
    setError(null);
    try {
      await dispatch({ RefreshDevices: null });
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setDeviceBusy(false);
    }
  }

  async function revokeDevice() {
    if (!deviceToRevoke || deviceToRevoke.current_device || deviceToRevoke.revoked) return;
    setDeviceBusy(true);
    setError(null);
    try {
      await dispatch({
        RevokeDevice: {
          account_id: deviceToRevoke.account_id,
          device_id: deviceToRevoke.device_id,
        },
      });
      setDeviceToRevoke(null);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setDeviceBusy(false);
    }
  }

  const connected = ownerClaimed && selectedRoom?.state === "Connected";
  const availableConnectionsHref = ownerClaimed ? connectionsHref : null;
  const activityLabel = sharedLiveActivityLabel(liveMembers, machineLabel, awaitingReply);

  return (
    <div className={sidebarCollapsed ? "finite-chat is-sidebar-collapsed" : "finite-chat"}>
      <ChatSidebar
        collapsed={sidebarCollapsed}
        isOpen={sidebarOpen}
        machineId={machineId}
        machineLabel={machineLabel}
        viewerEmail={viewerEmail}
        topics={canonicalTopics}
        previousTopics={previousTopics}
        selectedTopic={selectedTopic}
        selectedChat={selectedChat}
        liveMembers={state?.typing_members ?? []}
        connectionsHref={availableConnectionsHref}
        showSkills={showSkills}
        onCreateChat={(topic) => void createChat(topic)}
        onCreateTopic={() => setCreateTopicOpen(true)}
        onOpenChat={(topic, chat) => void openChat(topic, chat)}
        onOpenTopic={(topic) => void openTopic(topic)}
        onToggleCollapsed={() => setSidebarCollapsed((value) => !value)}
        onToggleOpen={() => setSidebarOpen((value) => !value)}
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
            <button
              type="button"
              className="ocean-icon-button finite-chat__sidebar-toggle"
              aria-label="Open chats"
              onClick={() => setSidebarOpen(true)}
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
            <ProductNavButton href={availableConnectionsHref} icon={PlugIcon} label="Connections" />
            <Button type="button" variant="ghost" size="sm" onClick={() => void openDevices()}>
              <MonitorSmartphoneIcon />
              <span>Devices</span>
            </Button>
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
                {!state && !error ? <ChatLoading label="Opening your chat…" /> : null}
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

              {error ? (
                <div className="finite-chat__send-error" role="alert">
                  <strong>Chat needs attention</strong>
                  <span>{error}</span>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => void (hasState && !ownerClaimed ? claimOwner() : load())}
                  >
                    <RotateCcwIcon />
                    {hasState && !ownerClaimed ? "Retry claim" : "Retry load"}
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
              <Button type="button" variant="outline" onClick={() => setCreateTopicOpen(false)}>Cancel</Button>
              <Button type="submit" disabled={!createTopicTitle.trim()}>Create topic</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

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

      <Dialog
        open={devicesOpen}
        onOpenChange={(open) => {
          setDevicesOpen(open);
          if (!open) setDeviceToRevoke(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{deviceToRevoke ? `Revoke ${deviceToRevoke.device_id}?` : "Your devices"}</DialogTitle>
            <DialogDescription>
              {deviceToRevoke
                ? "This permanently stops that Device from sending, receiving, or linking again with the same Device identity."
                : "Each linked browser or computer is a separate encrypted Device. Revoke one you no longer trust."}
            </DialogDescription>
          </DialogHeader>
          {deviceToRevoke ? (
            <DialogFooter>
              <Button type="button" variant="outline" disabled={deviceBusy} onClick={() => setDeviceToRevoke(null)}>
                Cancel
              </Button>
              <Button type="button" variant="destructive" disabled={deviceBusy} onClick={() => void revokeDevice()}>
                {deviceBusy ? "Revoking…" : "Revoke Device"}
              </Button>
            </DialogFooter>
          ) : (
            <div className="grid gap-2">
              {deviceBusy ? <ChatLoading label="Refreshing devices…" /> : null}
              {!deviceBusy && (state?.devices ?? []).length === 0 ? (
                <p className="text-sm text-muted-foreground">No linked Devices are visible yet.</p>
              ) : null}
              {(state?.devices ?? []).map((device) => (
                <div
                  key={`${device.account_id}:${device.device_id}`}
                  className="flex items-center gap-3 rounded-xl border border-border p-3"
                >
                  <MonitorSmartphoneIcon className="size-4 text-muted-foreground" />
                  <div className="min-w-0 flex-1">
                    <strong className="block truncate text-sm">{device.device_id}</strong>
                    <span className="text-xs text-muted-foreground">
                      {device.current_device
                        ? "This browser"
                        : device.revoked
                          ? "Revoked"
                          : device.active
                            ? `Active in ${device.room_count} ${pluralize("room", device.room_count)}`
                            : "Linked, not currently active"}
                    </span>
                  </div>
                  {!device.current_device && !device.revoked ? (
                    <Button type="button" variant="outline" size="sm" onClick={() => setDeviceToRevoke(device)}>
                      Revoke
                    </Button>
                  ) : null}
                </div>
              ))}
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}

function ChatSidebar({
  collapsed,
  connectionsHref,
  isOpen,
  liveMembers,
  machineId,
  machineLabel,
  onCreateChat,
  onCreateTopic,
  onOpenChat,
  onOpenTopic,
  onToggleCollapsed,
  onToggleOpen,
  selectedChat,
  selectedTopic,
  showSkills,
  topics,
  previousTopics,
  viewerEmail,
}: {
  collapsed: boolean;
  connectionsHref?: string | null;
  isOpen: boolean;
  liveMembers: HostedChatTypingMember[];
  machineId: string;
  machineLabel: string;
  onCreateChat: (topic: HostedChatTopic) => void;
  onCreateTopic: () => void;
  onOpenChat: (topic: HostedChatTopic, chat: HostedChatSummary) => void;
  onOpenTopic: (topic: HostedChatTopic) => void;
  onToggleCollapsed: () => void;
  onToggleOpen: () => void;
  selectedChat: HostedChatSummary | null;
  selectedTopic: HostedChatTopic | null;
  showSkills: boolean;
  topics: HostedChatTopic[];
  previousTopics: HostedChatTopic[];
  viewerEmail?: string | null;
}) {
  return (
    <aside className={`finite-chat__sidebar ${isOpen ? "is-open" : ""}`}>
      <div className="finite-chat__sidebar-top">
        <div className="finite-chat__brand"><FiniteBrand /></div>
        <button
          type="button"
          className="ocean-icon-button finite-chat__desktop-collapse-button"
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          aria-pressed={collapsed}
          onClick={onToggleCollapsed}
        >
          <PanelLeftIcon className="size-4" />
        </button>
        <button
          type="button"
          className="ocean-icon-button finite-chat__mobile-collapse-button"
          aria-label="Close sidebar"
          onClick={onToggleOpen}
        >
          <PanelLeftIcon className="size-4" />
        </button>
      </div>

      <nav className="finite-chat__sidebar-nav" aria-label="Topics and chats">
        <div className="finite-chat__sidebar-section-row">
          <span className="finite-chat__sidebar-section">Topics</span>
          <button type="button" className="ocean-icon-button" aria-label="New topic" onClick={onCreateTopic}>
            <PlusIcon className="size-3.5" />
          </button>
        </div>
        {topics.map((topic) => {
          const topicMembers = liveMembers.filter(
            (member) => !member.topic_id || member.topic_id === topic.topic_id
          );
          return (
            <div className="finite-chat__folder" key={`${topic.room_id}:${topic.topic_id}`}>
              <div className="finite-chat__folder-header">
                <button
                  type="button"
                  className={`finite-chat__folder-summary ${topic.topic_id === selectedTopic?.topic_id ? "is-active" : ""}`}
                  onClick={() => onOpenTopic(topic)}
                >
                  <span className="finite-chat__folder-main">
                  <span className="finite-chat__folder-icon" aria-hidden><HashIcon className="size-3.5" /></span>
                  <span className="finite-chat__folder-label">{topic.title}</span>
                  </span>
                  {topic.unread_count > 0 ? <span className="finite-chat__unread-count">{topic.unread_count}</span> : null}
                </button>
                <button
                  type="button"
                  className="finite-chat__topic-new-chat"
                  aria-label={`New chat in ${topic.title}`}
                  onClick={() => onCreateChat(topic)}
                >
                  <PlusIcon className="size-3.5" />
                </button>
              </div>
              <div className="finite-chat__folder-body">
                {topic.chats.map((chat) => {
                  const activity = activityForChat(topicMembers, topic, chat);
                  const active = topic.topic_id === selectedTopic?.topic_id && chat.chat_id === selectedChat?.chat_id;
                  return (
                    <button
                      key={chat.chat_id}
                      type="button"
                      className={[active ? "is-active" : "", activity ? "is-working" : ""].filter(Boolean).join(" ")}
                      aria-current={active ? "page" : undefined}
                      aria-busy={activity ? true : undefined}
                      onClick={() => onOpenChat(topic, chat)}
                    >
                      <ThreadActivityIndicator state={activity} />
                      <span className="finite-chat__thread-main">
                        <span className="finite-chat__thread-title">{chat.title || "New chat"}</span>
                        {chat.unread_count > 0 ? <span className="finite-chat__unread-count">{chat.unread_count}</span> : null}
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          );
        })}
        {previousTopics.length > 0 ? (
          <>
            <div className="finite-chat__sidebar-section-row">
              <span className="finite-chat__sidebar-section">Previous conversations</span>
            </div>
            {previousTopics.map((topic) => (
              <div className="finite-chat__folder" key={`previous:${topic.room_id}:${topic.topic_id}`}>
                <div className="finite-chat__folder-header">
                  <button
                    type="button"
                    className={`finite-chat__folder-summary ${topic.room_id === selectedTopic?.room_id && topic.topic_id === selectedTopic?.topic_id ? "is-active" : ""}`}
                    onClick={() => onOpenTopic(topic)}
                  >
                    <span className="finite-chat__folder-main">
                      <span className="finite-chat__folder-icon" aria-hidden><HashIcon className="size-3.5" /></span>
                      <span className="finite-chat__folder-label">{topic.title}</span>
                    </span>
                  </button>
                </div>
                <div className="finite-chat__folder-body">
                  {topic.chats.map((chat) => (
                    <button
                      key={chat.chat_id}
                      type="button"
                      className={topic.room_id === selectedTopic?.room_id && topic.topic_id === selectedTopic?.topic_id && chat.chat_id === selectedChat?.chat_id ? "is-active" : ""}
                      onClick={() => onOpenChat(topic, chat)}
                    >
                      <ThreadActivityIndicator state={null} />
                      <span className="finite-chat__thread-main">
                        <span className="finite-chat__thread-title">{chat.title || "New chat"}</span>
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            ))}
          </>
        ) : null}
      </nav>

      <button
        type="button"
        className="finite-chat__sidebar-new-chat-fab"
        disabled={!selectedTopic}
        onClick={() => selectedTopic && onCreateChat(selectedTopic)}
      >
        <PlusIcon className="size-4" />
        <span>New chat</span>
      </button>

      <div className="finite-chat__sidebar-footer">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button type="button" className="finite-chat__user-row">
              <span className="finite-chat__avatar" aria-hidden>{initials(viewerEmail || machineLabel)}</span>
              <span className="finite-chat__user-name">{viewerEmail || machineLabel}</span>
              <MoreHorizontalIcon className="size-4" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" side="top" sideOffset={8} className="finite-chat__app-menu">
            <DropdownMenuLabel className="finite-chat__app-menu-heading">Signed in as</DropdownMenuLabel>
            <div className="finite-chat__app-menu-account">
              <span className="finite-chat__avatar" aria-hidden>{initials(viewerEmail || machineLabel)}</span>
              <span>{viewerEmail || "Local development account"}</span>
            </div>
            <DropdownMenuSeparator className="finite-chat__app-menu-separator" />
            <AppMenuLink href={`/dashboard/machines/${encodeURIComponent(machineId)}`} icon={MonitorIcon} label="Agent" note="Status and recovery" />
            <AppMenuLink href={connectionsHref} icon={PlugIcon} label="Connections" note="Product access" />
            {showSkills ? <AppMenuLink href={`/dashboard/skills?machine=${encodeURIComponent(machineId)}`} icon={WrenchIcon} label="Skills" note="Managed capabilities" /> : null}
            <DropdownMenuSeparator className="finite-chat__app-menu-separator" />
            <DropdownMenuItem asChild className="finite-chat__app-menu-item">
              <Link href="/logout"><LogOutIcon /><span><strong>Sign out</strong><small>End this session</small></span></Link>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </aside>
  );
}

function ProductNavButton({ href, icon: Icon, label }: { href?: string | null; icon: LucideIcon; label: string }) {
  if (!href) {
    return (
      <Button type="button" variant="ghost" size="sm" disabled title={`${label} is not connected to Account Auth yet`}>
        <Icon />
        <span>{label}</span>
      </Button>
    );
  }
  return <Button asChild variant="ghost" size="sm"><Link href={href}><Icon /><span>{label}</span></Link></Button>;
}

function AppMenuLink({ href, icon: Icon, label, note }: { href?: string | null; icon: LucideIcon; label: string; note: string }) {
  if (!href) {
    return <DropdownMenuItem disabled className="finite-chat__app-menu-item"><Icon /><span><strong>{label}</strong><small>{note} · not configured</small></span></DropdownMenuItem>;
  }
  return <DropdownMenuItem asChild className="finite-chat__app-menu-item"><Link href={href}><Icon /><span><strong>{label}</strong><small>{note}</small></span></Link></DropdownMenuItem>;
}

function ThreadActivityIndicator({ state }: { state: string | null }) {
  return (
    <span className={`finite-chat__thread-indicator ${state ? `is-${state}` : ""}`} aria-hidden>
      {state ? <span className="finite-chat__thread-pulse" /> : null}
    </span>
  );
}

function activityForChat(members: HostedChatTypingMember[], topic: HostedChatTopic, chat: HostedChatSummary) {
  const member = members.find(
    (candidate) =>
      (!candidate.topic_id || candidate.topic_id === topic.topic_id)
      && (!candidate.chat_id || candidate.chat_id === chat.chat_id)
  );
  return member?.activity_kind ?? null;
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
  if (attachment.kind !== "Image") {
    return <a href={href} className="finite-chat__file-card"><FileTextIcon className="size-4" /><span>{attachment.filename}</span></a>;
  }
  return (
    <span className={compact ? "finite-chat__image-card is-compact" : "finite-chat__image-card"}>
      <a className="finite-chat__image-link" href={href} target="_blank" rel="noreferrer">
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img src={href} alt={attachment.filename} />
      </a>
      <span className="finite-chat__image-caption">
        <span>{attachment.filename}</span>
        <span className="finite-chat__image-actions">
          <a href={href} download={attachment.filename} aria-label={`Download ${attachment.filename}`}><DownloadIcon className="size-3.5" /></a>
          <ShareAttachmentButton href={href} name={attachment.filename} />
        </span>
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

async function chatRequest<T>(url: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (typeof init.body === "string") headers.set("content-type", "application/json");
  const response = await fetch(url, { ...init, cache: "no-store", headers });
  if (!response.ok) {
    const text = await response.text();
    try {
      const parsed = JSON.parse(text) as { error?: string };
      throw new HostedChatHttpError(
        parsed.error || text || `Chat returned ${response.status}`,
        response.status
      );
    } catch (error) {
      if (error instanceof SyntaxError) {
        throw new HostedChatHttpError(text || `Chat returned ${response.status}`, response.status);
      }
      throw error;
    }
  }
  return response.json() as Promise<T>;
}

class HostedChatHttpError extends Error {
  constructor(
    message: string,
    readonly status: number
  ) {
    super(message);
  }
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : CHAT_UNAVAILABLE_MESSAGE;
}
