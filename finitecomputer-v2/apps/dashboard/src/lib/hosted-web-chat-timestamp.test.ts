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
  const html = renderToStaticMarkup(createElement(TooltipProvider, null, createElement(MessageRow, {
    attachmentUrl: () => "",
    message: value,
    ownAccountId: "user",
    shareTitle: "Chat",
  })));
  return html.match(/<time\b[^>]*>(.*?)<\/time>/u)?.[1];
}

test("webchat renders agent and user timestamps in the viewer's timezone", (t) => {
  const originalTimezone = process.env.TZ;
  process.env.TZ = "America/New_York";
  t.after(() => {
    if (originalTimezone === undefined) delete process.env.TZ;
    else process.env.TZ = originalTimezone;
  });

  assert.equal(renderedTime(message()), "11:51 AM");
  assert.equal(renderedTime(message({ final_delivery: false })), "11:51 AM");
  assert.equal(renderedTime(message({ sender_account_id: "user", is_mine: true })), "11:51 AM");

  // Winter uses EST rather than the summer EDT offset from the screenshot.
  assert.equal(renderedTime(message({
    timestamp_unix_seconds: Date.parse("2026-01-22T15:51:00Z") / 1000,
  })), "10:51 AM");

  process.env.TZ = "Asia/Tokyo";
  assert.equal(renderedTime(message()), "12:51 AM");
  process.env.TZ = "UTC";
  assert.equal(renderedTime(message()), "3:51 PM");
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
