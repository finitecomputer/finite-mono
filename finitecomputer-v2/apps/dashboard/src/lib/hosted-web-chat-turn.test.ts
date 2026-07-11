import assert from "node:assert/strict";
import test from "node:test";

import { hasFinalRemoteResponse } from "@/lib/hosted-web-chat-turn";
import type { HostedChatMessage } from "@/lib/hosted-web-device";

test("activity gaps, commentary, and multi-tool progress do not finish a pending Hermes turn", () => {
  const messages = [
    message({ seq: 11, kind: "message", status: "complete" }),
    message({ seq: 12, kind: "tool", status: "running" }),
    message({ seq: 13, kind: "tool", status: "complete" }),
    message({ seq: 14, kind: "status", status: "complete" }),
    message({ seq: 15, kind: "message", status: "running" }),
  ];

  // Working activity can disappear between any of these snapshots. It is not
  // an input to terminal detection, so the pending turn remains latched.
  assert.equal(hasFinalRemoteResponse(messages, 10), false);
});

test("only a post-send final remote response finishes the Hermes turn", () => {
  assert.equal(
    hasFinalRemoteResponse(
      [message({ seq: 11, kind: "message", status: "complete", finalDelivery: true })],
      10
    ),
    true
  );
});

test("own and stale final deliveries do not finish the turn", () => {
  assert.equal(
    hasFinalRemoteResponse(
      [
        message({ seq: 10, kind: "message", status: "complete", finalDelivery: true }),
        message({
          seq: 11,
          kind: "message",
          status: "complete",
          finalDelivery: true,
          isMine: true,
        }),
      ],
      10
    ),
    false
  );
});

function message({
  seq,
  kind,
  status,
  finalDelivery = false,
  isMine = false,
}: {
  seq: number;
  kind: HostedChatMessage["kind"];
  status: HostedChatMessage["status"];
  finalDelivery?: boolean;
  isMine?: boolean;
}): HostedChatMessage {
  return {
    room_id: "room-1",
    seq,
    message_id: `message-${seq}`,
    sender_account_id: isMine ? "account-me" : "account-agent",
    sender_device_id: isMine ? "device-me" : "device-agent",
    sender_display_name: isMine ? "Me" : "Agent",
    text: "",
    display_content: "",
    rich_text_json: "",
    is_mine: isMine,
    media: [],
    kind,
    status,
    final_delivery: finalDelivery,
    timestamp_unix_seconds: 1,
    display_timestamp: "now",
  };
}
