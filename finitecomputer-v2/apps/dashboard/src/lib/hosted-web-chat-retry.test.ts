import assert from "node:assert/strict";
import test from "node:test";

import {
  initialHostedChatRetryDelay,
  runInitialHostedChatRetries,
  shouldRetryHostedChatRequest,
  waitForHostedChatRetry,
} from "@/lib/hosted-web-chat-retry";

test("initial chat load retries with bounded exponential backoff", () => {
  assert.deepEqual(
    [1, 2, 3, 4, 5, 6].map(initialHostedChatRetryDelay),
    [500, 1_000, 2_000, 4_000, 8_000, null]
  );
  assert.equal(initialHostedChatRetryDelay(0), null);
  assert.equal(initialHostedChatRetryDelay(1.5), null);
});

test("an initial chat retry wait is cancellable", async () => {
  const controller = new AbortController();
  const waiting = waitForHostedChatRetry(60_000, controller.signal);
  controller.abort();
  assert.equal(await waiting, false);
});

test("chat recovery retries only transient and network failures", () => {
  assert.equal(shouldRetryHostedChatRequest(null), true);
  assert.equal(shouldRetryHostedChatRequest(408), true);
  assert.equal(shouldRetryHostedChatRequest(429), true);
  assert.equal(shouldRetryHostedChatRequest(502), true);
  assert.equal(shouldRetryHostedChatRequest(401), false);
  assert.equal(shouldRetryHostedChatRequest(403), false);
  assert.equal(shouldRetryHostedChatRequest(404), false);
});

test("chat recovery stops after success without another request", async () => {
  const controller = new AbortController();
  const results = ["retry", "retry", "succeeded"] as const;
  const delays: number[] = [];
  let attempts = 0;

  const result = await runInitialHostedChatRetries(
    async () => results[attempts++] ?? "stop",
    controller.signal,
    async (delay) => {
      delays.push(delay);
      return true;
    }
  );

  assert.equal(result, "succeeded");
  assert.equal(attempts, 3);
  assert.deepEqual(delays, [500, 1_000]);
});

test("chat recovery does not retry a terminal response", async () => {
  const controller = new AbortController();
  let attempts = 0;
  const result = await runInitialHostedChatRetries(
    async () => {
      attempts += 1;
      return "stop";
    },
    controller.signal
  );
  assert.equal(result, "stop");
  assert.equal(attempts, 1);
});
