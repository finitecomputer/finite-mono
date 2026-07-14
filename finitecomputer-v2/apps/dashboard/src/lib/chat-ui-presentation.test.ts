import assert from "node:assert/strict";
import test from "node:test";

import {
  activitiesForChat,
  attachmentSendError,
  beginPendingChatTurn,
  isUserPrincipalMessage,
  liveActivityLabel,
  messagesForChat,
  pendingTurnIsComplete,
  pendingTurnMatchesSelection,
  selectedChat,
  transcriptItems,
  type AppState,
  type ChatMessage,
} from "@finite/chat-ui";

test("the shared projection selects one Room Topic Chat and scopes messages and activity", () => {
  const state = appState();
  const selection = selectedChat(state);

  assert.equal(selection.room?.room_id, "room-agent");
  assert.equal(selection.topic?.topic_id, "home");
  assert.equal(selection.chat?.chat_id, "home-chat");
  assert.deepEqual(messagesForChat(state, selection).map((message) => message.message_id), ["web-1"]);
  assert.deepEqual(activitiesForChat(state, selection).map((member) => member.device_id), ["agent"]);
});

test("same-account messages render as the user even when authored by another Device", () => {
  const state = appState();
  const electronMessage = message({
    messageId: "electron-2",
    seq: 2,
    senderAccountId: state.identity.account_id,
    senderDeviceId: "electron-alpha",
    isMine: false,
  });

  assert.equal(isUserPrincipalMessage(electronMessage, state.identity), true);
});

test("pending working state survives tools and same-account Device traffic until agent final delivery", () => {
  const state = appState();
  const selection = selectedChat(state);
  const turn = beginPendingChatTurn(selection, messagesForChat(state, selection));
  assert(turn);
  const sameAccountFinal = message({
    messageId: "electron-final",
    seq: 2,
    senderAccountId: state.identity.account_id,
    senderDeviceId: "electron-alpha",
    isMine: false,
    finalDelivery: true,
  });
  const tool = message({
    messageId: "tool-3",
    seq: 3,
    senderAccountId: "agent-account",
    senderDeviceId: "agent",
    kind: "tool",
    status: "complete",
  });

  assert.equal(
    pendingTurnIsComplete(turn, [...state.messages, sameAccountFinal, tool], state.identity.account_id),
    false
  );
  const final = message({
    messageId: "agent-final",
    seq: 4,
    senderAccountId: "agent-account",
    senderDeviceId: "agent",
    finalDelivery: true,
  });
  assert.equal(
    pendingTurnIsComplete(turn, [...state.messages, sameAccountFinal, tool, final], state.identity.account_id),
    true
  );
});

test("a pending turn follows its chat without leaking into another selected chat", () => {
  const state = appState();
  const selection = selectedChat(state);
  const turn = beginPendingChatTurn(selection, messagesForChat(state, selection));
  assert(turn);

  assert.equal(pendingTurnMatchesSelection(turn, selection), true);
  assert.equal(
    pendingTurnMatchesSelection(turn, {
      ...selection,
      chat: selection.chat ? { ...selection.chat, chat_id: "other-chat" } : null,
    }),
    false
  );
});

test("working presentation requires live agent activity and outranks typing", () => {
  assert.equal(liveActivityLabel([], "Sol"), null);
  assert.equal(
    liveActivityLabel(
      [
        {
          room_id: "room-agent",
          account_id: "agent-account",
          device_id: "agent-typing",
          display_name: "Sol",
          activity_kind: "typing",
        },
        {
          room_id: "room-agent",
          account_id: "agent-account",
          device_id: "agent-working",
          display_name: "Sol",
          activity_kind: "working",
        },
      ],
      "Sol"
    ),
    "Sol is working"
  );
});

test("the shared transcript hides status, groups tools, and collapses edits", () => {
  const status = message({
    messageId: "status-1",
    seq: 2,
    kind: "status",
    status: "running",
  });
  const tool = message({
    messageId: "tool-1",
    seq: 3,
    kind: "tool",
    status: "running",
  });
  const editedTool = {
    ...message({
      messageId: "tool-edit",
      seq: 4,
      kind: "message",
      status: "running",
    }),
    edit_of_message_id: tool.message_id,
    display_content: "tool finished",
  };
  const final = message({ messageId: "final", seq: 5, finalDelivery: true });

  const transcript = transcriptItems(
    [status, tool, editedTool, final],
    "user-account"
  );
  assert.equal(transcript.length, 2);
  assert.equal(transcript[0]?.type, "tools");
  if (transcript[0]?.type === "tools") {
    assert.equal(transcript[0].messages.length, 1);
    assert.equal(transcript[0].messages[0]?.message_id, tool.message_id);
    assert.equal(transcript[0].messages[0]?.display_content, "tool finished");
    assert.equal(transcript[0].messages[0]?.status, "complete");
  }
  assert.equal(transcript[1]?.type, "message");
});

test("a swallowed Core attachment failure remains a composer error", () => {
  assert.equal(
    attachmentSendError({
      status: "attachment unavailable",
      toast: "Attachment upload failed: blob service unavailable",
    }),
    "Attachment upload failed: blob service unavailable"
  );
  assert.equal(attachmentSendError({ status: "sent", toast: null }), null);
});

function appState(): AppState {
  return {
    rev: 1,
    identity: { account_id: "user-account", device_id: "hosted-web", account_secret_hex: "" },
    rooms: [
      {
        room_id: "room-agent",
        display_name: "Agent",
        state: "Connected",
        status: "connected",
        user_status_text: "Connected",
        last_message_preview: "hello",
        unread_count: 0,
        can_load_older: false,
        is_agent_chat: true,
      },
    ],
    selected_room_id: "room-agent",
    topics: [
      {
        room_id: "room-agent",
        topic_id: "home",
        title: "Home",
        last_message_preview: "hello",
        unread_count: 0,
        message_count: 1,
        created_seq: 0,
        updated_seq: 1,
        archived: false,
        active_chat_id: "home-chat",
        chats: [
          {
            chat_id: "home-chat",
            title: "Chat",
            last_message_preview: "hello",
            unread_count: 0,
            message_count: 1,
            started_seq: 0,
            updated_seq: 1,
            active: true,
          },
        ],
      },
    ],
    selected_topic_id: "home",
    selected_chat_id: "home-chat",
    active_profile_id: null,
    status: "ready",
    toast: null,
    messages: [
      message({
        messageId: "web-1",
        seq: 1,
        senderAccountId: "user-account",
        senderDeviceId: "hosted-web",
        isMine: true,
      }),
      { ...message({ messageId: "other-chat", seq: 2 }), chat_id: "other-chat" },
    ],
    media_gallery: null,
    room_details: {
      room_id: "room-agent",
      display_name: "Agent",
      state: "Connected",
      status: "connected",
      user_status_text: "Connected",
      media_item_count: 0,
      members: [],
      devices: [],
    },
    profiles: [],
    devices: [],
    typing_members: [
      {
        room_id: "room-agent",
        topic_id: "home",
        chat_id: "home-chat",
        account_id: "agent-account",
        device_id: "agent",
        display_name: "Agent",
        activity_kind: "working",
      },
      {
        room_id: "room-agent",
        topic_id: "home",
        chat_id: "other-chat",
        account_id: "agent-account",
        device_id: "agent-other-chat",
        display_name: "Agent",
        activity_kind: "working",
      },
    ],
    flow: {
      notice_text: null,
      notice_busy: false,
      scan_in_flight: false,
      scan_result: "none",
      image_upload_url: null,
    },
  };
}

function message({
  messageId,
  seq,
  senderAccountId = "agent-account",
  senderDeviceId = "agent",
  isMine = false,
  kind = "message",
  status = "complete",
  finalDelivery = false,
}: {
  messageId: string;
  seq: number;
  senderAccountId?: string;
  senderDeviceId?: string;
  isMine?: boolean;
  kind?: ChatMessage["kind"];
  status?: ChatMessage["status"];
  finalDelivery?: boolean;
}): ChatMessage {
  return {
    room_id: "room-agent",
    conversation_id: "home",
    chat_id: "home-chat",
    seq,
    message_id: messageId,
    sender_account_id: senderAccountId,
    sender_device_id: senderDeviceId,
    sender_display_name: senderAccountId === "agent-account" ? "Agent" : "Paul",
    text: messageId,
    display_content: messageId,
    kind,
    status,
    final_delivery: finalDelivery,
    is_mine: isMine,
    media: [],
    timestamp_unix_seconds: 1,
    display_timestamp: "now",
  };
}
