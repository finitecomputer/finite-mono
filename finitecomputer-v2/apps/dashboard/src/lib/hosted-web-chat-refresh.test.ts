import assert from "node:assert/strict";
import test from "node:test";

import {
  pendingChatRefreshAdvancesTranscript,
  pendingChatRefreshIsDue,
  type PendingChatRefreshTarget,
} from "@/lib/hosted-web-chat-refresh";
import type { HostedChatState } from "@/lib/hosted-web-device";

const target: PendingChatRefreshTarget = {
  room_id: "room",
  topic_id: "home",
  chat_id: "chat",
  after_seq: 4,
};

test("a pending refresh accepts newer transcript data at an equal revision", () => {
  const current = state(7, 4);
  const next = state(7, 5);

  assert.equal(
    pendingChatRefreshAdvancesTranscript(current, next, target),
    true
  );
});

test("a pending refresh ignores unchanged data and a chat that is no longer selected", () => {
  assert.equal(
    pendingChatRefreshAdvancesTranscript(state(7, 4), state(7, 4), target),
    false
  );
  assert.equal(
    pendingChatRefreshAdvancesTranscript(
      { ...state(7, 4), selected_chat_id: "other-chat" },
      state(7, 5),
      target
    ),
    false
  );
  assert.equal(
    pendingChatRefreshAdvancesTranscript(state(8, 4), state(7, 5), target),
    false
  );
});

test("a pending refresh waits after transcript progress and after its previous attempt", () => {
  assert.equal(pendingChatRefreshIsDue(21_999, 10_000, null), false);
  assert.equal(pendingChatRefreshIsDue(22_000, 10_000, null), true);
  assert.equal(pendingChatRefreshIsDue(30_000, 10_000, 25_000), false);
  assert.equal(pendingChatRefreshIsDue(37_000, 10_000, 25_000), true);
});

function state(rev: number, latestSeq: number): HostedChatState {
  return {
    rev,
    identity: {
      account_id: "user",
      device_id: "web",
      account_secret_hex: "",
    },
    rooms: [],
    selected_room_id: target.room_id,
    topics: [],
    selected_topic_id: target.topic_id,
    selected_chat_id: target.chat_id,
    active_profile_id: null,
    status: "ready",
    toast: null,
    messages: latestSeq > 0
      ? [
          {
            room_id: target.room_id,
            conversation_id: target.topic_id,
            chat_id: target.chat_id,
            seq: latestSeq,
            message_id: `message-${latestSeq}`,
            sender_account_id: "agent",
            sender_device_id: "agent-device",
            sender_display_name: "Agent",
            text: "message",
            display_content: "message",
            kind: "message",
            status: "complete",
            final_delivery: false,
            is_mine: false,
            media: [],
            timestamp_unix_seconds: 1,
            display_timestamp: "now",
          },
        ]
      : [],
    media_gallery: null,
    room_details: null,
    profiles: [],
    devices: [],
    typing_members: [],
    flow: {
      notice_text: null,
      notice_busy: false,
      scan_in_flight: false,
      scan_result: "none",
      image_upload_url: null,
    },
  };
}
