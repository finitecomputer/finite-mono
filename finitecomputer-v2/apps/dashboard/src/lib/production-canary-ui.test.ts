import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  CONNECTIONS_TIMEOUT_MESSAGE,
  ConnectionsPanel,
  connectionErrorMessage,
  connectionRequest,
} from "@/components/connections-panel";
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

test("Connections initial render keeps truthful disabled controls inspectable", () => {
  const html = renderToStaticMarkup(
    createElement(ConnectionsPanel, {
      machineId: "canary-agent",
      googleConfigured: true,
    })
  );
  assert.match(html, /Checking live connection status/u);
  assert.match(html, /Status unavailable/u);
  assert.match(html, /Use Finite Private/u);
  assert.match(html, /Telegram/u);
  assert.match(html, /Google Workspace/u);
  assert.doesNotMatch(html, /<a\b/u, "no external connection flow is live before status loads");
  assert.match(html, /<button[^>]+disabled/u);
});

test("Connections stops waiting at its browser deadline and offers a retry", async (t) => {
  const originalFetch = globalThis.fetch;
  const keepEventLoopAlive = setTimeout(() => {}, 100);
  globalThis.fetch = async (_input, init) =>
    new Promise<Response>((_resolve, reject) => {
      const signal = init?.signal;
      assert.ok(signal, "Connections requests must carry a deadline signal");
      signal.addEventListener("abort", () => reject(signal.reason), { once: true });
    });
  t.after(() => {
    clearTimeout(keepEventLoopAlive);
    globalThis.fetch = originalFetch;
  });

  let caught: unknown;
  try {
    await connectionRequest("/api/connections/machines/canary-agent", undefined, 5);
  } catch (error) {
    caught = error;
  }

  assert.ok(caught instanceof Error);
  assert.equal(connectionErrorMessage(caught), CONNECTIONS_TIMEOUT_MESSAGE);
  assert.match(CONNECTIONS_TIMEOUT_MESSAGE, /Try again/u);
});

test("Connections keeps safe server errors and hides unknown failures", () => {
  assert.equal(
    connectionErrorMessage(new Error("This agent is still starting. Try again shortly.")),
    "This agent is still starting. Try again shortly."
  );
  assert.equal(
    connectionErrorMessage(null),
    "Connections are unavailable right now."
  );
});
