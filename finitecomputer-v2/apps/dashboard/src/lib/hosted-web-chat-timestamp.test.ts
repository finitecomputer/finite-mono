import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { MessageRow } from "@/components/hosted-web-chat";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { HostedChatMessage } from "@/lib/hosted-web-device";

function message(overrides: Partial<HostedChatMessage> = {}): HostedChatMessage {
  return {
    room_id: "room",
    seq: 1,
    message_id: "message",
    sender_account_id: "agent",
    sender_device_id: "agent-device",
    sender_display_name: "Agent",
    text: "Hello",
    display_content: "Hello",
    is_mine: false,
    media: [],
    kind: "message",
    status: "complete",
    final_delivery: true,
    timestamp_unix_seconds: Date.parse("2026-09-22T15:51:00Z") / 1000,
    display_timestamp: "3:51 PM",
    ...overrides,
  };
}

function renderedTime(value: HostedChatMessage) {
  const row = createElement(MessageRow, {
    attachmentUrl: () => "",
    message: value,
    ownAccountId: "user",
    shareTitle: "Chat",
  });
  const html = renderToStaticMarkup(createElement(TooltipProvider, null, row));
  return html.match(/<time\b[^>]*>(.*?)<\/time>/u)?.[1];
}

// Expected wall-clock hours are explicit; their label follows the test runner's locale.
function clockTime(hour: number, minute: number) {
  return new Date(2026, 0, 1, hour, minute).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
}

test("webchat renders agent and user timestamps in the viewer's timezone", (t) => {
  const originalTimezone = process.env.TZ;
  process.env.TZ = "America/New_York";
  t.after(() => {
    if (originalTimezone === undefined) delete process.env.TZ;
    else process.env.TZ = originalTimezone;
  });

  assert.equal(renderedTime(message()), clockTime(11, 51));
  assert.equal(renderedTime(message({ final_delivery: false })), clockTime(11, 51));
  assert.equal(renderedTime(message({ sender_account_id: "user", is_mine: true })), clockTime(11, 51));

  // Winter uses EST rather than the summer EDT offset from the screenshot.
  assert.equal(renderedTime(message({
    timestamp_unix_seconds: Date.parse("2026-01-22T15:51:00Z") / 1000,
  })), clockTime(10, 51));

  process.env.TZ = "Asia/Tokyo";
  assert.equal(renderedTime(message()), clockTime(0, 51));
  process.env.TZ = "UTC";
  assert.equal(renderedTime(message()), clockTime(15, 51));
});

test("webchat preserves delivery labels in place of the user timestamp", () => {
  for (const [server_delivery, label] of [
    ["Undelivered", "Sending…"],
    ["Delivered", "Delivered"],
    [{ Failed: { reason: "Unavailable" } }, "Not delivered"],
  ] as const) {
    assert.equal(renderedTime(message({
      sender_account_id: "user",
      is_mine: true,
      outbound_delivery: { local_send: "Sent", server_delivery },
    })), label);
  }
});

test("missing or invalid message times stay blank", () => {
  for (const timestamp_unix_seconds of [0, -1, NaN, Infinity, Number.MAX_SAFE_INTEGER]) {
    assert.equal(renderedTime(message({ timestamp_unix_seconds })), "");
  }
});
