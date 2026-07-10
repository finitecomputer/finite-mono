import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { ConnectionsPanel } from "@/components/connections-panel";
import {
  CHAT_INVALID_UPDATE_MESSAGE,
  CHAT_TOPIC_DESCRIPTION,
  CHAT_UNAVAILABLE_MESSAGE,
  CHAT_WAITING_FOR_AGENT_MESSAGE,
} from "@/lib/chat-product-copy";
import { connectionsReadiness } from "@/lib/connections-readiness";

test("normal chat copy does not expose hosted-device or transport language", () => {
  const copy = [
    CHAT_INVALID_UPDATE_MESSAGE,
    CHAT_TOPIC_DESCRIPTION,
    CHAT_UNAVAILABLE_MESSAGE,
    CHAT_WAITING_FOR_AGENT_MESSAGE,
  ];
  for (const message of copy) {
    assert.doesNotMatch(
      message,
      /hosted|device|transport|stream|relay|runtime|workos|protocol|mls|encrypted room/iu
    );
  }
});

test("Connections stays closed until claimed status loads", () => {
  assert.equal(connectionsReadiness(false, null), "loading");
  assert.equal(connectionsReadiness(false, "Owner claim failed"), "error");
  assert.equal(connectionsReadiness(true, null), "ready");
  assert.equal(
    connectionsReadiness(true, "A later refresh failed"),
    "ready",
    "a status can exist only after the initial typed claim succeeded"
  );
});

test("Connections initial render exposes no connection controls", () => {
  const html = renderToStaticMarkup(
    createElement(ConnectionsPanel, {
      machineId: "canary-agent",
      googleConfigured: true,
    })
  );
  assert.match(html, /Preparing your connections/u);
  assert.doesNotMatch(html, /<(?:a|button|input|form)\b/u);
});
