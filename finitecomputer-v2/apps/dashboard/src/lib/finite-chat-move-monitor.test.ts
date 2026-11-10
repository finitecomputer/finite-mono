import assert from "node:assert/strict";
import { test } from "node:test";

import { FINITE_CHAT_MOVE_ROUTE, startFiniteChatMoveMonitor } from "./finite-chat-move-monitor";

const settle = () => new Promise((resolve) => setImmediate(resolve));

function answering(body: unknown, ok = true) {
  const asked: string[] = [];
  const fetcher = (async (url: string) => {
    asked.push(url);
    return { ok, json: async () => body } as Response;
  }) as unknown as typeof fetch;
  return { asked, fetcher };
}

test("an open dashboard leaves for finite.chat once the route says so", async () => {
  const { asked, fetcher } = answering({ redirect: "https://finite.chat/" });
  const went: string[] = [];
  const stop = startFiniteChatMoveMonitor({ replace: (url) => went.push(url), fetcher });
  await settle();
  stop();
  assert.deepEqual(asked, [FINITE_CHAT_MOVE_ROUTE]);
  assert.deepEqual(went, ["https://finite.chat/"]);
});

test("it stays when the person hasn't moved, the route fails, or the answer isn't a web address", async () => {
  for (const [body, ok] of [
    [{ redirect: null }, true],
    [{ redirect: "https://finite.chat/" }, false],
    [{ redirect: "javascript:alert(1)" }, true],
    [{}, true],
  ] as const) {
    const { fetcher } = answering(body, ok);
    const went: string[] = [];
    const stop = startFiniteChatMoveMonitor({ replace: (url) => went.push(url), fetcher });
    await settle();
    stop();
    assert.deepEqual(went, [], JSON.stringify(body));
  }
});
