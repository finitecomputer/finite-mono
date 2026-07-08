import { FormEvent, KeyboardEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  BotIcon,
  CheckIcon,
  ChevronRightIcon,
  FileIcon,
  HashIcon,
  ImageIcon,
  KeyRoundIcon,
  Loader2Icon,
  MessageCircleIcon,
  MoreHorizontalIcon,
  PaperclipIcon,
  RefreshCwIcon,
  SendIcon,
  ShieldCheckIcon,
  SparklesIcon,
  SquarePenIcon,
  UserPlusIcon,
  UsersIcon,
  XIcon,
} from "lucide-react";
import { FiniteBrand } from "@/components/finite-brand";
import {
  AppRoomMemberSummary,
  AppRoomSummary,
  AppState,
  AppProfileSummary,
  AppTopicSummary,
  AppTypingMember,
  ChatMediaAttachment,
  ChatMediaKind,
  ChatMessage,
  OutboundAttachment,
  dispatch,
  getState,
  resolveDaemonUrl,
  subscribeToUpdates,
} from "./daemon";
import { LegacyFiniteChatSourceMarker } from "./legacy/LegacyFiniteChatSourceMarker";

type DesktopIdentityStatus = {
  secureStorageAvailable: boolean;
  hasStoredAccountSecret: boolean;
};

type DesktopOnboardingStatus = {
  completed: boolean;
};

const MAX_COMPOSER_ATTACHMENT_BYTES = 25 * 1024 * 1024;
const HOME_TOPIC_ID = "home";

type ComposerAttachment = OutboundAttachment & {
  id: string;
  size: number;
};

type LocalPendingMessage = {
  local_id: string;
  room_id: string;
  conversation_id: string | null;
  chat_id: string | null;
  text: string;
  attachments: Pick<ComposerAttachment, "id" | "filename" | "mime_type" | "kind" | "size">[];
  state: "sending" | "failed";
  created_at: string;
};

export function App() {
  const [daemonUrl, setDaemonUrl] = useState<string | null>(null);
  const [state, setState] = useState<AppState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [composer, setComposer] = useState("");
  const [agentProfileInput, setAgentProfileInput] = useState("");
  const [createMode, setCreateMode] = useState<"chat" | "topic" | null>(null);
  const [createTitle, setCreateTitle] = useState("");
  const [participantOpen, setParticipantOpen] = useState(false);
  const [participantInput, setParticipantInput] = useState("");
  const [accountMenuOpen, setAccountMenuOpen] = useState(false);
  const [identityStatus, setIdentityStatus] = useState<DesktopIdentityStatus | null>(null);
  const [onboardingStatus, setOnboardingStatus] = useState<DesktopOnboardingStatus | null>(null);
  const [identitySecret, setIdentitySecret] = useState("");
  const [identityBusy, setIdentityBusy] = useState(false);
  const [composerAttachments, setComposerAttachments] = useState<ComposerAttachment[]>([]);
  const [localPendingMessages, setLocalPendingMessages] = useState<LocalPendingMessage[]>([]);
  const [awaitingReplyRoomIds, setAwaitingReplyRoomIds] = useState<string[]>([]);
  const agentProfileInputRef = useRef<HTMLInputElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const transcriptRef = useRef<HTMLElement | null>(null);
  const lastDesktopTargetUrlRef = useRef<{ url: string; timestamp: number } | null>(null);
  const typingRoomRef = useRef<string | null>(null);
  const typingStopTimerRef = useRef<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    resolveDaemonUrl()
      .then((url) => {
        if (!cancelled) {
          setDaemonUrl(url);
        }
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setError(errorMessage(reason));
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const refresh = useCallback(async () => {
    if (!daemonUrl) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      setState(await getState(daemonUrl));
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  }, [daemonUrl]);

  useEffect(() => {
    if (!daemonUrl) {
      return;
    }
    void refresh();
    return subscribeToUpdates(
      daemonUrl,
      (next) => {
        setError(null);
        setState(next);
      },
      (reason) => setError(reason.message)
    );
  }, [daemonUrl, refresh]);

  useEffect(() => {
    if (!daemonUrl || state) {
      return;
    }
    const timer = window.setInterval(() => void refresh(), 1200);
    return () => window.clearInterval(timer);
  }, [daemonUrl, refresh, state]);

  const run = useCallback(
    async (action: Parameters<typeof dispatch>[1]) => {
      if (!daemonUrl) {
        return null;
      }
      setBusy(true);
      setError(null);
      try {
        const next = await dispatch(daemonUrl, action);
        setState(next);
        return next;
      } catch (reason) {
        setError(errorMessage(reason));
        return null;
      } finally {
        setBusy(false);
      }
    },
    [daemonUrl]
  );

  const runQuiet = useCallback(
    async (action: Parameters<typeof dispatch>[1]) => {
      if (!daemonUrl) {
        return null;
      }
      try {
        const next = await dispatch(daemonUrl, action);
        setState(next);
        return next;
      } catch {
        return null;
      }
    },
    [daemonUrl]
  );

  const runComposerAction = useCallback(
    async (action: Parameters<typeof dispatch>[1]) => {
      if (!daemonUrl) {
        return null;
      }
      setError(null);
      try {
        const next = await dispatch(daemonUrl, action);
        setState(next);
        return next;
      } catch (reason) {
        setError(errorMessage(reason));
        return null;
      }
    },
    [daemonUrl]
  );

  const loadDesktopState = useCallback(async () => {
    if (!window.finiteChatDesktop) {
      setOnboardingStatus({ completed: true });
      return;
    }
    const [identity, onboarding] = await Promise.all([
      window.finiteChatDesktop.identityStatus(),
      window.finiteChatDesktop.onboardingStatus(),
    ]);
    setIdentityStatus(identity);
    setOnboardingStatus(onboarding);
  }, []);

  useEffect(() => {
    void loadDesktopState();
  }, [loadDesktopState]);

  const handleDesktopTargetUrl = useCallback(
    (url: string | null | undefined) => {
      const value = url?.trim();
      if (!value) {
        return;
      }
      const last = lastDesktopTargetUrlRef.current;
      const now = Date.now();
      if (last?.url === value && now - last.timestamp < 2000) {
        return;
      }
      lastDesktopTargetUrlRef.current = { url: value, timestamp: now };
      void run({ ScanTarget: { value } });
    },
    [run]
  );

  useEffect(() => {
    if (!window.finiteChatDesktop || !daemonUrl) {
      return;
    }
    const unsubscribe = window.finiteChatDesktop.onTargetUrl(handleDesktopTargetUrl);
    void window.finiteChatDesktop
      .consumePendingTargetUrl()
      .then(handleDesktopTargetUrl)
      .catch((reason: unknown) => setError(errorMessage(reason)));
    return unsubscribe;
  }, [daemonUrl, handleDesktopTargetUrl]);

  const selectedRoom = useMemo(
    () => state?.rooms.find((room) => room.room_id === state.selected_room_id) ?? null,
    [state]
  );
  const selectedTopic = useMemo(
    () =>
      state?.topics.find(
        (topic) => topic.room_id === state.selected_room_id && topic.topic_id === state.selected_topic_id
      ) ?? null,
    [state]
  );
  const selectedChat = useMemo(
    () => selectedTopic?.chats.find((chat) => chat.chat_id === state?.selected_chat_id) ?? selectedTopic?.chats[0] ?? null,
    [selectedTopic, state?.selected_chat_id]
  );
  const topicsByRoom = useMemo(() => {
    const grouped = new Map<string, AppTopicSummary[]>();
    for (const topic of state?.topics ?? []) {
      if (topic.archived) {
        continue;
      }
      const topics = grouped.get(topic.room_id) ?? [];
      topics.push(topic);
      grouped.set(topic.room_id, topics);
    }
    for (const topics of grouped.values()) {
      topics.sort((left, right) => {
        if (left.topic_id === HOME_TOPIC_ID) {
          return -1;
        }
        if (right.topic_id === HOME_TOPIC_ID) {
          return 1;
        }
        return right.updated_seq - left.updated_seq || left.title.localeCompare(right.title);
      });
    }
    return grouped;
  }, [state?.topics]);
  const activeProfile = useMemo(
    () => state?.profiles.find((profile) => profile.account_id === state.active_profile_id) ?? null,
    [state?.active_profile_id, state?.profiles]
  );
  const selectedMessages = state?.messages ?? [];
  const selectedMembers = selectedRoom?.members ?? [];
  const agentRooms = state?.rooms.filter((room) => room.is_agent_chat) ?? [];
  const selectedRoomHasCounterparty = selectedRoom
    ? selectedRoom.is_agent_chat || selectedMembers.some((member) => !member.current_device)
    : false;
  const canSendToSelectedRoom =
    Boolean(selectedRoom) && selectedRoom?.state === "Connected" && selectedRoomHasCounterparty;
  const selectedRoomNeedsAgent = Boolean(selectedRoom) && !selectedRoomHasCounterparty;
  const statusText = state ? (error ?? state.flow.notice_text ?? state.toast ?? state.status) : "starting daemon";
  const shortAccount = state?.identity.account_id ? shortId(state.identity.account_id) : "not connected";
  const showOnboarding = window.finiteChatDesktop ? onboardingStatus?.completed === false : false;
  const selectedLiveMembers = useMemo(
    () => state?.typing_members.filter((member) => member.room_id === selectedRoom?.room_id) ?? [],
    [selectedRoom?.room_id, state?.typing_members]
  );
  const visiblePendingMessages = useMemo(
    () =>
      localPendingMessages.filter(
        (message) =>
          message.room_id === selectedRoom?.room_id &&
          (selectedTopic
            ? message.conversation_id === selectedTopic.topic_id && message.chat_id === (selectedChat?.chat_id ?? null)
            : message.conversation_id === null)
      ),
    [localPendingMessages, selectedChat?.chat_id, selectedRoom?.room_id, selectedTopic]
  );
  const hasComposerContent = Boolean(composer.trim() || composerAttachments.length > 0);
  const awaitingSelectedAgent =
    Boolean(selectedRoom?.is_agent_chat) &&
    Boolean(selectedRoom?.room_id && awaitingReplyRoomIds.includes(selectedRoom.room_id)) &&
    selectedLiveMembers.length === 0;

  const focusAgentInput = useCallback(() => {
    window.requestAnimationFrame(() => {
      const input = agentProfileInputRef.current;
      input?.closest(".finite-chat__agent-panel")?.scrollIntoView({ block: "center", behavior: "smooth" });
      input?.focus();
    });
  }, []);

  useEffect(() => {
    const transcript = transcriptRef.current;
    if (!transcript) {
      return;
    }
    transcript.scrollTo({ top: transcript.scrollHeight, behavior: "smooth" });
  }, [
    selectedMessages.length,
    visiblePendingMessages.length,
    selectedLiveMembers.length,
    selectedRoom?.room_id,
    selectedTopic?.topic_id,
    selectedChat?.chat_id,
  ]);

  useEffect(() => {
    if (!selectedRoom || selectedMessages.length === 0) {
      return;
    }
    const last = selectedMessages[selectedMessages.length - 1];
    if (!last?.is_mine) {
      setAwaitingReplyRoomIds((roomIds) =>
        roomIds.includes(selectedRoom.room_id) ? roomIds.filter((roomId) => roomId !== selectedRoom.room_id) : roomIds
      );
    }
  }, [selectedMessages, selectedRoom?.room_id]);

  useEffect(() => {
    if (!selectedRoom || selectedRoom.state !== "Connected" || !selectedMessages.some((message) => !message.is_mine)) {
      return;
    }
    const timer = window.setTimeout(() => {
      void runQuiet({ MarkRoomRead: { room_id: selectedRoom.room_id } });
    }, 350);
    return () => window.clearTimeout(timer);
  }, [runQuiet, selectedMessages.length, selectedRoom?.room_id, selectedRoom?.state]);

  useEffect(() => {
    return () => {
      if (typingStopTimerRef.current !== null) {
        window.clearTimeout(typingStopTimerRef.current);
      }
      if (typingRoomRef.current) {
        void runQuiet({ SetTyping: { room_id: typingRoomRef.current, is_typing: false } });
      }
    };
  }, [runQuiet, selectedRoom?.room_id]);

  function stopTyping(roomId = typingRoomRef.current) {
    if (typingStopTimerRef.current !== null) {
      window.clearTimeout(typingStopTimerRef.current);
      typingStopTimerRef.current = null;
    }
    if (roomId) {
      typingRoomRef.current = null;
      void runQuiet({ SetTyping: { room_id: roomId, is_typing: false } });
    }
  }

  function noteTyping(nextValue: string) {
    if (!selectedRoom || !canSendToSelectedRoom) {
      return;
    }
    if (!nextValue.trim()) {
      stopTyping(selectedRoom.room_id);
      return;
    }
    if (typingRoomRef.current !== selectedRoom.room_id) {
      typingRoomRef.current = selectedRoom.room_id;
      void runQuiet({ SetTyping: { room_id: selectedRoom.room_id, is_typing: true } });
    }
    if (typingStopTimerRef.current !== null) {
      window.clearTimeout(typingStopTimerRef.current);
    }
    typingStopTimerRef.current = window.setTimeout(() => stopTyping(selectedRoom.room_id), 2200);
  }

  function handleComposerChange(value: string) {
    setComposer(value);
    noteTyping(value);
  }

  async function submitComposer(event: FormEvent) {
    event.preventDefault();
    const text = composer.trim();
    const attachments = composerAttachments;
    if ((!text && attachments.length === 0) || !state) {
      return;
    }
    if (!selectedRoom) {
      setError(agentRooms.length > 0 ? "Select an agent chat before sending." : "Connect Hermes before sending.");
      focusAgentInput();
      return;
    }
    if (text === "/new" && attachments.length === 0) {
      const topicForNewChat =
        selectedTopic ??
        (topicsByRoom.get(selectedRoom.room_id) ?? []).find((topic) => topic.topic_id === HOME_TOPIC_ID) ??
        (topicsByRoom.get(selectedRoom.room_id) ?? [])[0] ??
        null;
      if (!topicForNewChat) {
        setError("Create a topic before starting another chat.");
        return;
      }
      stopTyping(selectedRoom.room_id);
      setComposer("");
      await runComposerAction({
        StartTopicChat: {
          room_id: topicForNewChat.room_id,
          topic_id: topicForNewChat.topic_id,
          reason: null,
        },
      });
      return;
    }
    if (!canSendToSelectedRoom) {
      setError(
        selectedRoom.state === "Connected"
          ? "This chat has no other member. Connect Hermes before sending."
          : "This chat is not ready for messages yet."
      );
      focusAgentInput();
      return;
    }
    if (selectedTopic && attachments.length > 0 && !selectedChat) {
      setError("Start a chat in this topic before attaching files.");
      return;
    }
    stopTyping(selectedRoom.room_id);
    const pendingId = `local-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    setComposer("");
    setComposerAttachments([]);
    setLocalPendingMessages((messages) => [
      ...messages,
      {
        local_id: pendingId,
        room_id: selectedRoom.room_id,
        conversation_id: selectedTopic?.topic_id ?? null,
        chat_id: selectedChat?.chat_id ?? null,
        text,
        attachments: attachments.map(({ id, filename, mime_type, kind, size }) => ({ id, filename, mime_type, kind, size })),
        state: "sending",
        created_at: "Sending",
      },
    ]);
    if (selectedRoom.is_agent_chat) {
      setAwaitingReplyRoomIds((roomIds) => (roomIds.includes(selectedRoom.room_id) ? roomIds : [...roomIds, selectedRoom.room_id]));
    }
    const next = attachments.length
      ? selectedTopic && selectedChat
        ? await runComposerAction({
            SendChatAttachments: {
              room_id: selectedTopic.room_id,
              topic_id: selectedTopic.topic_id,
              chat_id: selectedChat.chat_id,
              attachments: attachments.map(({ filename, mime_type, kind, bytes }) => ({ filename, mime_type, kind, bytes })),
              caption: text,
              reply_to_message_id: null,
            },
          })
        : await runComposerAction({
            SendAttachments: {
              room_id: selectedRoom.room_id,
              attachments: attachments.map(({ filename, mime_type, kind, bytes }) => ({ filename, mime_type, kind, bytes })),
              caption: text,
              reply_to_message_id: null,
            },
          })
      : selectedTopic
        ? selectedChat
          ? await runComposerAction({
              SendChatMessage: {
                room_id: selectedTopic.room_id,
                topic_id: selectedTopic.topic_id,
                chat_id: selectedChat.chat_id,
                text,
              },
            })
          : await runComposerAction({
              SendTopicMessage: {
                room_id: selectedTopic.room_id,
                topic_id: selectedTopic.topic_id,
                text,
              },
            })
        : await runComposerAction({ SendMessage: { room_id: selectedRoom.room_id, text } });
    if (next) {
      setLocalPendingMessages((messages) => messages.filter((message) => message.local_id !== pendingId));
    } else {
      setLocalPendingMessages((messages) =>
        messages.map((message) => (message.local_id === pendingId ? { ...message, state: "failed", created_at: "Not sent" } : message))
      );
    }
  }

  async function handleAttachmentFiles(files: FileList | null) {
    if (!files || files.length === 0) {
      return;
    }
    const next: ComposerAttachment[] = [];
    for (const file of Array.from(files)) {
      if (file.size > MAX_COMPOSER_ATTACHMENT_BYTES) {
        setError(`${file.name} is larger than ${formatBytes(MAX_COMPOSER_ATTACHMENT_BYTES)}.`);
        continue;
      }
      const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
      next.push({
        id: `${file.name}-${file.size}-${file.lastModified}-${Math.random().toString(36).slice(2)}`,
        filename: file.name,
        mime_type: file.type || "application/octet-stream",
        kind: mediaKindForFile(file),
        bytes,
        size: file.size,
      });
    }
    if (next.length > 0) {
      setComposerAttachments((attachments) => [...attachments, ...next].slice(0, 8));
    }
    if (fileInputRef.current) {
      fileInputRef.current.value = "";
    }
  }

  function removeComposerAttachment(id: string) {
    setComposerAttachments((attachments) => attachments.filter((attachment) => attachment.id !== id));
  }

  async function submitAgentProfile(event: FormEvent) {
    event.preventDefault();
    const added = await connectProfileTarget(agentProfileInput, selectedRoom);
    if (added) {
      setAgentProfileInput("");
    }
  }

  async function submitCreate(event?: FormEvent) {
    event?.preventDefault();
    if (!createMode) {
      return;
    }
    const title = createTitle.trim();
    setCreateTitle("");
    setCreateMode(null);
    if (createMode === "chat") {
      const topicForNewChat =
        selectedTopic ??
        (selectedRoom ? (topicsByRoom.get(selectedRoom.room_id) ?? []).find((topic) => topic.topic_id === HOME_TOPIC_ID) : null) ??
        (selectedRoom ? (topicsByRoom.get(selectedRoom.room_id) ?? [])[0] : null) ??
        null;
      if (topicForNewChat) {
        await run({
          StartTopicChat: {
            room_id: topicForNewChat.room_id,
            topic_id: topicForNewChat.topic_id,
            reason: title || null,
          },
        });
        return;
      }
      await run({ CreateRoom: { display_name: title || "New chat" } });
      return;
    }
    if (!selectedRoom) {
      setError("Select a chat before creating a topic.");
      return;
    }
    await run({ CreateTopic: { room_id: selectedRoom.room_id, title: title || "New topic" } });
  }

  async function submitParticipant(event: FormEvent) {
    event.preventDefault();
    const added = await connectProfileTarget(participantInput, selectedRoom);
    if (added) {
      setParticipantInput("");
      setParticipantOpen(false);
    }
  }

  async function connectProfileTarget(value: string, roomOverride: AppRoomSummary | null) {
    const target = value.trim();
    if (!target) {
      return false;
    }
    const scanned = await run({ ScanTarget: { value: target } });
    const profile = scanned ? profileFromState(scanned, scanned.active_profile_id) : null;
    if (!profile) {
      setError("Paste a Finite npub or profile link.");
      return false;
    }
    const roomForAdd = roomOverride?.state === "Connected" ? roomOverride : null;
    const next = roomForAdd
      ? await run({ AddRoomMembers: { room_id: roomForAdd.room_id, profiles: [profile] } })
      : await run({
          StartProfileChat: {
            profile,
            display_name: `Chat with ${profile.display_name}`,
          },
        });
    if (!next) {
      return false;
    }
    if (next.status === "chat unavailable" || next.toast) {
      setError(next.toast ?? "That npub is not available for an encrypted chat yet.");
      return false;
    }
    return true;
  }

  async function syncRuntime() {
    await run({ StartRuntime: null });
  }

  async function finishOnboarding() {
    if (window.finiteChatDesktop) {
      setOnboardingStatus(await window.finiteChatDesktop.completeOnboarding());
    } else {
      setOnboardingStatus({ completed: true });
    }
    void refresh();
  }

  async function importDesktopIdentity(secret: string) {
    if (!window.finiteChatDesktop || !secret.trim()) {
      return;
    }
    setIdentityBusy(true);
    setError(null);
    try {
      setIdentityStatus(await window.finiteChatDesktop.importAccountSecret(secret));
      setIdentitySecret("");
      setAccountMenuOpen(false);
      setState(null);
      if (window.finiteChatDesktop) {
        setOnboardingStatus(await window.finiteChatDesktop.completeOnboarding());
      }
      window.setTimeout(() => void refresh(), 700);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setIdentityBusy(false);
    }
  }

  async function submitAccountImport(event: FormEvent) {
    event.preventDefault();
    await importDesktopIdentity(identitySecret);
  }

  async function clearDesktopIdentity() {
    if (!window.finiteChatDesktop) {
      return;
    }
    setIdentityBusy(true);
    setError(null);
    try {
      setIdentityStatus(await window.finiteChatDesktop.clearAccountSecret());
      setState(null);
      window.setTimeout(() => void refresh(), 700);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setIdentityBusy(false);
    }
  }

  return (
    <div className="finite-chat finite-chat--electron">
      <LegacyFiniteChatSourceMarker />
      <aside className="finite-chat__sidebar">
        <div className="finite-chat__sidebar-top">
          <div className="finite-chat__brand">
            <FiniteBrand />
          </div>
          <button
            type="button"
            className="ocean-icon-button"
            aria-label="Refresh"
            onClick={() => void syncRuntime()}
            disabled={busy}
          >
            {busy ? <Loader2Icon className="finite-chat__spin" aria-hidden /> : <RefreshCwIcon aria-hidden />}
          </button>
        </div>

        <nav className="finite-chat__sidebar-nav" aria-label="Chat navigation">
          <div className="finite-chat__sidebar-actions">
            <button
              type="button"
              className="finite-chat__sidebar-action"
              onClick={() => {
                setCreateMode("chat");
                setCreateTitle("");
              }}
              disabled={busy}
            >
              <SquarePenIcon aria-hidden />
              <span>New chat</span>
            </button>
            <button
              type="button"
              className="finite-chat__sidebar-action"
              onClick={() => {
                setCreateMode("topic");
                setCreateTitle("");
              }}
              disabled={busy || !selectedRoom}
            >
              <HashIcon aria-hidden />
              <span>New topic</span>
            </button>
          </div>

          {createMode ? (
            <form className="finite-chat__sidebar-create" onSubmit={submitCreate}>
              <input
                value={createTitle}
                onChange={(event) => setCreateTitle(event.target.value)}
                placeholder={
                  createMode === "chat"
                    ? selectedTopic
                      ? "Chat note"
                      : "Chat name"
                    : selectedRoom
                      ? "Topic name"
                      : "Select a chat first"
                }
                autoFocus
                disabled={busy || (createMode === "topic" && !selectedRoom)}
              />
              <button
                type="submit"
                className="ocean-icon-button"
                aria-label={createMode === "chat" ? "Create chat" : "Create topic"}
                disabled={busy || (createMode === "topic" && !selectedRoom)}
              >
                <CheckIcon aria-hidden />
              </button>
              <button
                type="button"
                className="ocean-icon-button"
                aria-label="Cancel"
                onClick={() => {
                  setCreateMode(null);
                  setCreateTitle("");
                }}
              >
                <XIcon aria-hidden />
              </button>
            </form>
          ) : null}

          <div className="finite-chat__folder-body finite-chat__topic-list">
            {(state?.rooms ?? []).map((room) => {
              const roomTopics = topicsByRoom.get(room.room_id) ?? [];
              if (roomTopics.length === 0) {
                return (
                  <RoomThreadButton
                    key={room.room_id}
                    room={room}
                    active={room.room_id === state?.selected_room_id}
                    onClick={() => void run({ OpenRoom: { room_id: room.room_id } })}
                  />
                );
              }
              return (
                <div className="finite-chat__sidebar-topic-room-group" key={room.room_id}>
                  {state && state.rooms.length > 1 ? (
                    <button
                      type="button"
                      className="finite-chat__sidebar-topic-room-label"
                      onClick={() => void run({ OpenRoom: { room_id: room.room_id } })}
                    >
                      <ThreadActivityIndicator active={room.state === "Joining" || room.state === "WaitingForApproval"} />
                      <span>{room.display_name}</span>
                    </button>
                  ) : null}
                  {roomTopics.map((topic) => (
                    <div className="finite-chat__sidebar-topic-group" key={`${topic.room_id}:${topic.topic_id}`}>
                      <TopicThreadButton
                        topic={topic}
                        active={topic.room_id === state?.selected_room_id && topic.topic_id === state?.selected_topic_id}
                        onClick={() => void run({ OpenTopic: { room_id: topic.room_id, topic_id: topic.topic_id } })}
                        onNewChat={() =>
                          void run({
                            StartTopicChat: {
                              room_id: topic.room_id,
                              topic_id: topic.topic_id,
                              reason: null,
                            },
                          })
                        }
                      />
                      {topic.chats.length > 0 ? (
                        <div className="finite-chat__sidebar-segment-list">
                          {topic.chats.map((chat, index) => (
                            <TopicChatButton
                              key={chat.chat_id}
                              chat={chat}
                              index={index}
                              active={
                                topic.room_id === state?.selected_room_id &&
                                topic.topic_id === state?.selected_topic_id &&
                                chat.chat_id === state?.selected_chat_id
                              }
                              onClick={() =>
                                void run({
                                  OpenChat: {
                                    room_id: topic.room_id,
                                    topic_id: topic.topic_id,
                                    chat_id: chat.chat_id,
                                  },
                                })
                              }
                            />
                          ))}
                        </div>
                      ) : null}
                    </div>
                  ))}
                </div>
              );
            })}
            {state && state.rooms.length === 0 ? (
              <div className="finite-chat__thread-empty">
                <MessageCircleIcon aria-hidden />
                <span>No chats</span>
              </div>
            ) : null}
          </div>
        </nav>

        <div className="finite-chat__sidebar-footer">
          {accountMenuOpen ? (
            <div className="finite-chat__account-menu">
              <div className="finite-chat__account-heading">
                <KeyRoundIcon aria-hidden />
                <span>Desktop identity</span>
              </div>
              <div className="finite-chat__account-id">
                <strong>{shortAccount}</strong>
                <small>
                  {identityStatus?.hasStoredAccountSecret
                    ? "Imported key in secure storage"
                    : "Local key on this Mac"}
                </small>
              </div>
              <form className="finite-chat__account-import" onSubmit={submitAccountImport}>
                <input
                  value={identitySecret}
                  onChange={(event) => setIdentitySecret(event.target.value)}
                  placeholder="nsec or 64-char secret"
                  type="password"
                  disabled={identityBusy || identityStatus?.secureStorageAvailable === false}
                />
                <button
                  type="submit"
                  className="finite-chat__command-button"
                  disabled={!identitySecret.trim() || identityBusy || identityStatus?.secureStorageAvailable === false}
                >
                  Save
                </button>
              </form>
              {identityStatus?.hasStoredAccountSecret ? (
                <button
                  className="finite-chat__account-link"
                  type="button"
                  onClick={() => void clearDesktopIdentity()}
                  disabled={identityBusy}
                >
                  Use local identity
                </button>
              ) : null}
              {identityStatus?.secureStorageAvailable === false ? (
                <div className="finite-chat__account-warning">Secure store unavailable</div>
              ) : null}
            </div>
          ) : null}
          <button type="button" className="finite-chat__user-row" onClick={() => setAccountMenuOpen((open) => !open)}>
            <span className="finite-chat__avatar" aria-hidden>
              {initials(state?.identity.device_id ?? "Desktop")}
            </span>
            <span className="finite-chat__user-name">{state?.identity.device_id ?? "Desktop"}</span>
            <MoreHorizontalIcon aria-hidden />
          </button>
        </div>
      </aside>

      <section className="finite-chat__workspace">
        <header className="finite-chat__topbar">
          <div className="finite-chat__identity">
            <strong>{selectedTopic?.title ?? selectedRoom?.display_name ?? "Finite Chat"}</strong>
            <span>
              <span className={`finite-chat__status-dot ${error ? "is-error" : state ? "is-running" : ""}`} aria-hidden />
              {selectedRoom ? selectedRoom.user_status_text || selectedRoom.state : statusText}
            </span>
          </div>
          <div className="finite-chat__topbar-actions">
            {state?.flow.scan_in_flight ? <Loader2Icon className="finite-chat__spin" aria-hidden /> : null}
            {selectedMembers.length > 0 ? <MembersPill members={selectedMembers} /> : null}
            {selectedRoom ? (
              <button
                type="button"
                className="ocean-icon-button"
                aria-label="Add participant"
                onClick={() => setParticipantOpen((open) => !open)}
                disabled={busy}
              >
                <UserPlusIcon aria-hidden />
              </button>
            ) : null}
          </div>
        </header>

        {participantOpen ? (
          <ParticipantPanel
            activeProfile={activeProfile}
            busy={busy}
            selectedRoom={selectedRoom}
            value={participantInput}
            onClose={() => setParticipantOpen(false)}
            onSubmit={submitParticipant}
            onValueChange={setParticipantInput}
          />
        ) : null}

        {error ? (
          <section className="finite-chat__notice finite-chat__notice--inline is-error">
            <strong>Daemon</strong>
            <span>{error}</span>
          </section>
        ) : null}

        {state && (!selectedRoom || selectedRoomNeedsAgent || selectedRoom.state !== "Connected") ? (
          <AgentConnectionPanel
            value={agentProfileInput}
            busy={busy}
            inputRef={agentProfileInputRef}
            selectedRoom={selectedRoom}
            hasAgentRoom={agentRooms.length > 0}
            onSubmit={submitAgentProfile}
            onValueChange={setAgentProfileInput}
          />
        ) : null}

        <div className="finite-chat__split">
          <main className="finite-chat__main">
            <section className="finite-chat__scroll" ref={transcriptRef}>
              <div className="finite-chat__messages">
                {selectedMessages.map((message) => (
                  <MessageRow key={`${message.room_id}:${message.message_id}`} message={message} />
                ))}
                {visiblePendingMessages.map((message) => (
                  <PendingMessageRow key={message.local_id} message={message} />
                ))}
                {awaitingSelectedAgent ? <LiveActivityIndicator label="Waiting for Hermes" /> : null}
                {selectedLiveMembers.length > 0 ? <LiveActivityIndicator members={selectedLiveMembers} /> : null}
                {!state ? (
                  <EmptyState title="Starting daemon" busy />
                ) : selectedMessages.length === 0 && visiblePendingMessages.length === 0 ? (
                  <EmptyState title={selectedTopic?.title ?? selectedRoom?.display_name ?? "Finite Chat"} />
                ) : null}
              </div>
            </section>

            <form className="finite-chat__composer-wrap" onSubmit={submitComposer}>
              <div className="finite-chat__composer">
                <input
                  ref={fileInputRef}
                  className="finite-chat__file-input"
                  type="file"
                  multiple
                  onChange={(event) => void handleAttachmentFiles(event.currentTarget.files)}
                />
                {composerAttachments.length > 0 ? (
                  <div className="finite-chat__attachment-tray">
                    {composerAttachments.map((attachment) => (
                      <button
                        key={attachment.id}
                        type="button"
                        className="finite-chat__attachment-chip"
                        onClick={() => removeComposerAttachment(attachment.id)}
                        title="Remove attachment"
                      >
                        {attachment.kind === "Image" ? <ImageIcon aria-hidden /> : <FileIcon aria-hidden />}
                        <span>
                          <strong>{attachment.filename}</strong>
                          <small>{formatBytes(attachment.size)}</small>
                        </span>
                        <XIcon aria-hidden />
                      </button>
                    ))}
                  </div>
                ) : null}
                <textarea
                  value={composer}
                  onChange={(event) => handleComposerChange(event.target.value)}
                  placeholder={composerPlaceholder(state, selectedRoom, selectedTopic, selectedRoomHasCounterparty)}
                  disabled={!state || busy || !canSendToSelectedRoom}
                  autoFocus
                  onBlur={() => stopTyping()}
                  onKeyDown={handleComposerKeyDown}
                />
                <div className="finite-chat__composer-actions">
                  <div className="finite-chat__composer-left">
                    <button
                      type="button"
                      className="finite-chat__tool-button"
                      aria-label="Attach file"
                      disabled={!state || busy || !canSendToSelectedRoom}
                      title="Attach file"
                      onClick={() => fileInputRef.current?.click()}
                    >
                      <PaperclipIcon aria-hidden />
                    </button>
                    <button type="button" className="finite-chat__command-button" disabled>
                      <SparklesIcon aria-hidden />
                      {selectedRoom?.is_agent_chat ? "Hermes" : selectedRoomHasCounterparty ? "Room" : "No agent"}
                    </button>
                  </div>
                  <div className="finite-chat__composer-right">
                    <button
                      type="submit"
                      className="finite-chat__send-button"
                      aria-label="Send message"
                      disabled={!state || !hasComposerContent || busy || !canSendToSelectedRoom}
                    >
                      <SendIcon aria-hidden />
                    </button>
                  </div>
                </div>
              </div>
            </form>
          </main>
        </div>
      </section>

      {showOnboarding ? (
        <DesktopOnboarding
          accountId={shortAccount}
          busy={identityBusy || busy}
          error={error}
          identityStatus={identityStatus}
          onImport={(secret) => importDesktopIdentity(secret)}
          onUseLocal={() => void finishOnboarding()}
        />
      ) : null}
    </div>
  );

  function handleComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.currentTarget.form?.requestSubmit();
    }
  }
}

function DesktopOnboarding({
  accountId,
  busy,
  error,
  identityStatus,
  onImport,
  onUseLocal,
}: {
  accountId: string;
  busy: boolean;
  error: string | null;
  identityStatus: DesktopIdentityStatus | null;
  onImport: (secret: string) => Promise<void>;
  onUseLocal: () => void;
}) {
  const [secret, setSecret] = useState("");

  async function submit(event: FormEvent) {
    event.preventDefault();
    await onImport(secret);
  }

  return (
    <div className="finite-chat__onboarding" role="dialog" aria-modal="true" aria-labelledby="finite-chat-onboarding-title">
      <section className="finite-chat__onboarding-panel">
        <div className="finite-chat__onboarding-brand">
          <FiniteBrand />
          <span>Desktop</span>
        </div>
        <div className="finite-chat__onboarding-copy">
          <h1 id="finite-chat-onboarding-title">Finite Chat</h1>
          <p>
            This desktop keeps a Finite identity locally. New installs create one automatically; import only when
            this device should use an existing npub.
          </p>
        </div>

        <button type="button" className="finite-chat__onboarding-choice" onClick={onUseLocal} disabled={busy}>
          <ShieldCheckIcon aria-hidden />
          <span>
            <strong>{identityStatus?.hasStoredAccountSecret ? "Continue with imported account" : "Continue with this device"}</strong>
            <small>
              {identityStatus?.hasStoredAccountSecret ? "Key stored in macOS secure storage" : `Local identity ${accountId}`}
            </small>
          </span>
        </button>

        <form className="finite-chat__onboarding-import" onSubmit={submit}>
          <label htmlFor="finite-chat-secret">Use an existing npub</label>
          <div>
            <input
              id="finite-chat-secret"
              value={secret}
              onChange={(event) => setSecret(event.target.value)}
              placeholder="nsec or 64-char secret"
              type="password"
              disabled={busy || identityStatus?.secureStorageAvailable === false}
            />
            <button
              type="submit"
              className="finite-chat__send-button"
              aria-label="Import account"
              disabled={!secret.trim() || busy || identityStatus?.secureStorageAvailable === false}
            >
              {busy ? <Loader2Icon className="finite-chat__spin" aria-hidden /> : <KeyRoundIcon aria-hidden />}
            </button>
          </div>
          {identityStatus?.secureStorageAvailable === false ? <span>Secure store unavailable</span> : null}
        </form>

        {error ? (
          <div className="finite-chat__onboarding-error">
            <strong>Daemon</strong>
            <span>{error}</span>
          </div>
        ) : null}
      </section>
    </div>
  );
}

function AgentConnectionPanel({
  busy,
  hasAgentRoom,
  inputRef,
  onSubmit,
  onValueChange,
  selectedRoom,
  value,
}: {
  busy: boolean;
  hasAgentRoom: boolean;
  inputRef: { current: HTMLInputElement | null };
  onSubmit: (event: FormEvent) => void;
  onValueChange: (value: string) => void;
  selectedRoom: AppRoomSummary | null;
  value: string;
}) {
  const copy = agentConnectionCopy(selectedRoom, hasAgentRoom);
  return (
    <section className="finite-chat__agent-panel">
      <div className="finite-chat__agent-panel-icon" aria-hidden>
        {selectedRoom?.state === "WaitingForApproval" ? (
          <Loader2Icon className="finite-chat__spin" />
        ) : (
          <BotIcon />
        )}
      </div>
      <div className="finite-chat__agent-panel-copy">
        <strong>{copy.title}</strong>
        <span>{copy.body}</span>
      </div>
      <form className="finite-chat__agent-panel-form" onSubmit={onSubmit}>
        <input
          ref={inputRef}
          value={value}
          onChange={(event) => onValueChange(event.target.value)}
          placeholder="Paste Hermes npub or profile link"
          disabled={busy}
        />
        <button type="submit" className="finite-chat__send-button" aria-label="Connect Hermes" disabled={!value.trim() || busy}>
          <UserPlusIcon aria-hidden />
        </button>
      </form>
    </section>
  );
}

function ParticipantPanel({
  activeProfile,
  busy,
  onClose,
  onSubmit,
  onValueChange,
  selectedRoom,
  value,
}: {
  activeProfile: AppProfileSummary | null;
  busy: boolean;
  onClose: () => void;
  onSubmit: (event: FormEvent) => void;
  onValueChange: (value: string) => void;
  selectedRoom: AppRoomSummary | null;
  value: string;
}) {
  const submitLabel = selectedRoom?.state === "Connected" ? "Add to chat" : "Start chat";
  return (
    <section className="finite-chat__participant-panel">
      <div className="finite-chat__participant-heading">
        <span>
          <UserPlusIcon aria-hidden />
          <strong>Add to chat</strong>
        </span>
        <button type="button" className="ocean-icon-button" aria-label="Close" onClick={onClose}>
          <XIcon aria-hidden />
        </button>
      </div>
      <form className="finite-chat__participant-form" onSubmit={onSubmit}>
        <input
          value={value}
          onChange={(event) => onValueChange(event.target.value)}
          placeholder="npub1... or nprofile1..."
          disabled={busy}
        />
        <button type="submit" className="finite-chat__command-button" disabled={!value.trim() || busy}>
          <UserPlusIcon aria-hidden />
          {submitLabel}
        </button>
      </form>
      {activeProfile ? (
        <div className="finite-chat__participant-profile">
          <span className="finite-chat__avatar" aria-hidden>
            {initials(activeProfile.display_name)}
          </span>
          <span>
            <strong>{activeProfile.display_name}</strong>
            <small>{activeProfile.npub}</small>
          </span>
        </div>
      ) : null}
    </section>
  );
}

function RoomThreadButton({
  active,
  onClick,
  room,
}: {
  active: boolean;
  onClick: () => void;
  room: AppRoomSummary;
}) {
  const working = room.state === "Joining" || room.state === "WaitingForApproval";
  return (
    <button
      type="button"
      aria-busy={working ? true : undefined}
      className={[active ? "is-active" : "", working ? "is-working" : ""].filter(Boolean).join(" ")}
      onClick={onClick}
    >
      <ThreadActivityIndicator active={working} />
      <span className="finite-chat__thread-main">
        <span className="finite-chat__thread-title">{room.display_name}</span>
        <span className="finite-chat__thread-time">{room.unread_count > 0 ? room.unread_count : room.state}</span>
      </span>
    </button>
  );
}

function TopicThreadButton({
  active,
  onNewChat,
  onClick,
  topic,
}: {
  active: boolean;
  onNewChat: () => void;
  onClick: () => void;
  topic: AppTopicSummary;
}) {
  return (
    <div className={`finite-chat__topic-thread-row ${active ? "is-active" : ""}`}>
      <button type="button" className="finite-chat__topic-thread finite-chat__topic-header" onClick={onClick}>
        <HashIcon aria-hidden />
        <span className="finite-chat__thread-main">
          <span className="finite-chat__thread-title">{topic.title}</span>
          <span className="finite-chat__thread-time">{topic.unread_count > 0 ? topic.unread_count : topic.message_count}</span>
        </span>
      </button>
      <button type="button" className="finite-chat__topic-new-chat" aria-label={`New chat in ${topic.title}`} onClick={onNewChat}>
        <SquarePenIcon aria-hidden />
      </button>
    </div>
  );
}

function TopicChatButton({
  active,
  chat,
  index,
  onClick,
}: {
  active: boolean;
  chat: AppTopicSummary["chats"][number];
  index: number;
  onClick: () => void;
}) {
  return (
    <button type="button" className={`finite-chat__topic-segment ${active ? "is-active" : ""}`} onClick={onClick}>
      <MessageCircleIcon aria-hidden />
      <span className="finite-chat__thread-main">
        <span className="finite-chat__thread-title">{chat.title || `Chat ${index + 1}`}</span>
        <span className="finite-chat__thread-time">{chat.unread_count > 0 ? chat.unread_count : chat.message_count}</span>
      </span>
    </button>
  );
}

function agentConnectionCopy(
  selectedRoom: AppRoomSummary | null,
  hasAgentRoom: boolean
) {
  const admissionDetail = selectedRoom ? roomAdmissionDetail(selectedRoom) : null;
  if (admissionDetail) {
    return {
      title: "Hermes admission needs attention",
      body: admissionDetail,
    };
  }
  if (selectedRoom?.state === "WaitingForApproval" || selectedRoom?.state === "Joining") {
    return {
      title: "Waiting for Hermes",
      body: "Hermes needs to publish key packages and receive this room's Welcome before messages can flow.",
    };
  }
  if (selectedRoom && !selectedRoom.is_agent_chat) {
    return {
      title: "No agent in this chat",
      body: "Paste the Hermes npub to add it to this room.",
    };
  }
  if (hasAgentRoom) {
    return {
      title: "Select an agent chat",
      body: "Hermes is connected in another room. Select that room or start a new topic there.",
    };
  }
  return {
    title: "Connect Hermes",
    body: "Paste the npub or profile link for a local or hosted Hermes runtime.",
  };
}

function roomAdmissionDetail(room: AppRoomSummary) {
  if (room.state !== "WaitingForApproval" && room.state !== "Joining") {
    return null;
  }
  const status = room.status.trim();
  if (!status) {
    return null;
  }
  const normalized = status.toLowerCase();
  if (
    normalized === "requesting room admission" ||
    normalized === "waiting for room admission" ||
    normalized === "joining"
  ) {
    return null;
  }
  return status.replace(/^client error:\s*/i, "");
}

function composerPlaceholder(
  state: AppState | null,
  selectedRoom: AppRoomSummary | null,
  selectedTopic: AppTopicSummary | null,
  selectedRoomHasCounterparty: boolean
) {
  if (!state) {
    return "Starting daemon";
  }
  if (!selectedRoom) {
    return "Connect Hermes to chat";
  }
  if (roomAdmissionDetail(selectedRoom)) {
    return "Hermes admission needs attention";
  }
  if (selectedRoom.state === "WaitingForApproval" || selectedRoom.state === "Joining") {
    return "Waiting for Hermes to admit this device";
  }
  if (!selectedRoomHasCounterparty) {
    return "Connect Hermes before sending";
  }
  return `Message ${selectedTopic?.title ?? selectedRoom.display_name}`;
}

function ThreadActivityIndicator({ active }: { active: boolean }) {
  return (
    <span className={`finite-chat__thread-indicator ${active ? "is-thinking" : ""}`} aria-hidden>
      {active ? <span className="finite-chat__thread-pulse" /> : <MessageCircleIcon />}
    </span>
  );
}

function MembersPill({ members }: { members: AppRoomMemberSummary[] }) {
  const visible = members.slice(0, 3);
  return (
    <div className="finite-chat__members-pill" title={members.map((member) => member.display_name).join(", ")}>
      {visible.map((member) => (
        <span key={`${member.account_id}:${member.device_id}`} className="finite-chat__avatar" aria-hidden>
          {initials(member.display_name)}
        </span>
      ))}
    </div>
  );
}

function LiveActivityIndicator({ label, members = [] }: { label?: string; members?: AppTypingMember[] }) {
  const displayLabel = label ?? liveActivityLabel(members);
  return (
    <div className="finite-chat__live-activity" aria-live="polite">
      <span className="finite-chat__live-dots" aria-hidden>
        <i />
        <i />
        <i />
      </span>
      <span>{displayLabel}</span>
    </div>
  );
}

function PendingMessageRow({ message }: { message: LocalPendingMessage }) {
  return (
    <article className={`finite-chat__message finite-chat__message--user finite-chat__message--pending ${message.state === "failed" ? "is-failed" : ""}`}>
      <div>
        {message.text ? <p>{message.text}</p> : null}
        {message.attachments.length > 0 ? (
          <div className="finite-chat__message-attachments">
            {message.attachments.map((attachment) => (
              <div key={attachment.id} className="finite-chat__message-attachment">
                {attachment.kind === "Image" ? <ImageIcon aria-hidden /> : <FileIcon aria-hidden />}
                <span>
                  <strong>{attachment.filename}</strong>
                  <small>{formatBytes(attachment.size)}</small>
                </span>
              </div>
            ))}
          </div>
        ) : null}
        <time className="finite-chat__message-time">{message.created_at}</time>
      </div>
    </article>
  );
}

function MessageRow({ message }: { message: ChatMessage }) {
  const content = message.display_content || message.text;
  if (message.is_mine) {
    return (
      <article className="finite-chat__message finite-chat__message--user">
        <div>
          {content ? <p>{content}</p> : null}
          <MessageAttachments media={message.media} />
          <time className="finite-chat__message-time">{deliveryText(message) ?? message.display_timestamp}</time>
        </div>
      </article>
    );
  }

  return (
    <article className="finite-chat__message finite-chat__message--agent">
      <div className="finite-chat__assistant-text">
        {content ? <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown> : null}
        <MessageAttachments media={message.media} />
      </div>
      <time className="finite-chat__message-time">
        {message.sender_display_name} · {message.display_timestamp}
      </time>
    </article>
  );
}

function MessageAttachments({ media }: { media?: ChatMediaAttachment[] }) {
  if (!media || media.length === 0) {
    return null;
  }
  return (
    <div className="finite-chat__message-attachments">
      {media.map((attachment) => {
        const previewUrl = attachment.local_path ? `file://${attachment.local_path}` : attachment.url || "";
        const canPreviewImage = attachment.kind === "Image" && previewUrl;
        return (
          <div key={attachment.attachment_id} className="finite-chat__message-attachment">
            {canPreviewImage ? <img src={previewUrl} alt="" /> : attachment.kind === "Image" ? <ImageIcon aria-hidden /> : <FileIcon aria-hidden />}
            <span>
              <strong>{attachment.filename}</strong>
              <small>{attachmentLabel(attachment)}</small>
            </span>
          </div>
        );
      })}
    </div>
  );
}

function EmptyState({ busy, title }: { busy?: boolean; title: string }) {
  return (
    <div className="finite-chat__empty finite-chat__empty--solo">
      <span className="finite-chat__empty-logo" aria-hidden>
        {busy ? <Loader2Icon className="finite-chat__spin" /> : <MessageCircleIcon />}
      </span>
      <h1>
        <span className="finite-chat__empty-title">{title}</span>
        <span className="finite-chat__empty-type-cursor" aria-hidden />
      </h1>
    </div>
  );
}

function mediaKindForFile(file: File): ChatMediaKind {
  if (file.type.startsWith("image/")) {
    return "Image";
  }
  if (file.type.startsWith("video/")) {
    return "Video";
  }
  if (file.type.startsWith("audio/")) {
    return "VoiceNote";
  }
  return "File";
}

function liveActivityLabel(members: AppTypingMember[]) {
  const working = members.find((member) => member.activity_kind === "working");
  const thinking = members.find((member) => member.activity_kind === "thinking");
  const typing = members.find((member) => member.activity_kind === "typing");
  const member = working ?? thinking ?? typing ?? members[0];
  const name = member?.display_name || "Someone";
  if (member?.activity_kind === "working") {
    return `${name} is working`;
  }
  if (member?.activity_kind === "thinking") {
    return `${name} is thinking`;
  }
  return `${name} is typing`;
}

function deliveryText(message: ChatMessage) {
  const delivery = message.outbound_delivery;
  if (!delivery) {
    return message.read_receipt?.display_text || null;
  }
  if (typeof delivery.server_delivery === "object" && "Failed" in delivery.server_delivery) {
    return `Not delivered: ${delivery.server_delivery.Failed.reason}`;
  }
  if (delivery.server_delivery === "Undelivered") {
    return "Sending...";
  }
  return message.read_receipt?.display_text || "Delivered";
}

function attachmentLabel(attachment: ChatMediaAttachment) {
  if (attachment.download_progress_per_mille !== null && attachment.download_progress_per_mille !== undefined) {
    return "Downloading";
  }
  if (attachment.upload_progress_per_mille !== null && attachment.upload_progress_per_mille !== undefined) {
    return "Uploading";
  }
  return attachment.mime_type || attachment.kind;
}

function formatBytes(bytes: number) {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function initials(value: string) {
  const trimmed = value.trim();
  if (!trimmed) {
    return "FC";
  }
  const parts = trimmed.split(/\s+/).slice(0, 2);
  return parts.map((part) => part[0]?.toUpperCase()).join("");
}

function shortId(value: string) {
  if (value.length <= 14) {
    return value;
  }
  return `${value.slice(0, 8)}...${value.slice(-4)}`;
}

function profileFromState(state: AppState, accountId: string | null | undefined) {
  if (!accountId) {
    return null;
  }
  return state.profiles.find((profile) => profile.account_id === accountId) ?? null;
}

function errorMessage(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason);
}
