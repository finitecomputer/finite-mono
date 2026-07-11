import assert from "node:assert/strict";
import test from "node:test";

import type { AppAction, AppState } from "@finite/chat-ui";
import { ChatProductController } from "../../../../../finitechat/packages/finitechat-chat-ui/src/react/controller";

import { createHostedWebChatTransport } from "@/lib/hosted-web-chat-transport";

test("hosted transport maps shared state, actions, and uploads onto one machine API", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => {
    globalThis.fetch = originalFetch;
  });

  const requests: Array<{ input: string; init: RequestInit }> = [];
  let rev = 0;
  globalThis.fetch = (async (input, init = {}) => {
    requests.push({ input: String(input), init });
    return Response.json(appState(++rev));
  }) as typeof fetch;

  const transport = createHostedWebChatTransport("agent/a b");
  const action: AppAction = {
    SendMessage: { room_id: "room-1", text: "hello from web" },
  };
  const file = new File(["hello"], "hello.txt", { type: "text/plain" });

  assert.equal((await transport.load()).rev, 1);
  assert.equal((await transport.dispatch(action)).rev, 2);
  assert.equal(
    (
      await transport.upload({
        room_id: "room-1",
        topic_id: "home",
        chat_id: "chat-1",
        reply_to_message_id: "message-1",
        caption: "A note",
        files: [file],
      })
    ).rev,
    3
  );

  const apiBase = "/api/chat/machines/agent%2Fa%20b/hosted-device";
  assert.deepEqual(
    requests.map(({ input }) => input),
    [`${apiBase}/state`, `${apiBase}/actions`, `${apiBase}/attachments`]
  );

  assert.equal(requests[0]!.init.cache, "no-store");
  assert.equal(new Headers(requests[0]!.init.headers).get("content-type"), "application/json");
  assert.equal(requests[1]!.init.method, "POST");
  assert.deepEqual(JSON.parse(String(requests[1]!.init.body)), action);

  const upload = requests[2]!;
  assert.equal(upload.init.method, "POST");
  assert.equal(new Headers(upload.init.headers).get("content-type"), null);
  assert.ok(upload.init.body instanceof FormData);
  assert.equal(upload.init.body.get("room_id"), "room-1");
  assert.equal(upload.init.body.get("topic_id"), "home");
  assert.equal(upload.init.body.get("chat_id"), "chat-1");
  assert.equal(upload.init.body.get("reply_to_message_id"), "message-1");
  assert.equal(upload.init.body.get("caption"), "A note");
  assert.equal(upload.init.body.get("files"), file);
});

test("hosted EventSource resets each generation and ignores closed generations", (context) => {
  const originalEventSource = globalThis.EventSource;
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  const timers: Array<() => void> = [];

  FakeEventSource.instances = [];
  globalThis.EventSource = FakeEventSource as unknown as typeof EventSource;
  globalThis.setTimeout = ((callback: TimerHandler) => {
    assert.equal(typeof callback, "function");
    timers.push(callback as () => void);
    return timers.length as unknown as ReturnType<typeof setTimeout>;
  }) as typeof setTimeout;
  globalThis.clearTimeout = (() => {}) as typeof clearTimeout;
  context.after(() => {
    globalThis.EventSource = originalEventSource;
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  });

  const states: number[] = [];
  const errors: string[] = [];
  let resets = 0;
  const transport = createHostedWebChatTransport("agent-1");
  const unsubscribe = transport.subscribe(
    (state) => states.push(state.rev),
    (error) => errors.push(error.message),
    () => {
      resets += 1;
    }
  );

  assert.equal(resets, 1);
  assert.equal(FakeEventSource.instances.length, 1);
  const first = FakeEventSource.instances[0]!;
  assert.equal(
    first.url,
    "/api/chat/machines/agent-1/hosted-device/updates"
  );

  first.emit("state", { data: JSON.stringify(appState(7)) });
  first.emit("state", { data: "not json" });
  assert.deepEqual(states, [7]);
  assert.deepEqual(errors, ["Chat received an update it could not read."]);

  first.emit("error", {});
  assert.equal(first.closed, true);
  assert.equal(timers.length, 1);
  assert.deepEqual(errors, [
    "Chat received an update it could not read.",
    "Reconnecting…",
  ]);

  timers.shift()!();
  assert.equal(resets, 2);
  assert.equal(FakeEventSource.instances.length, 2);
  const second = FakeEventSource.instances[1]!;
  first.emit("state", { data: JSON.stringify(appState(8)) });
  second.emit("state", { data: JSON.stringify(appState(1)) });
  assert.deepEqual(states, [7, 1], "a new generation accepts its authoritative baseline");

  unsubscribe();
  assert.equal(second.closed, true);
  second.emit("error", {});
  assert.equal(timers.length, 0, "a disposed subscription does not reconnect");
});

test("starting a new hosted subscription closes the previous stream", (context) => {
  const originalEventSource = globalThis.EventSource;
  FakeEventSource.instances = [];
  globalThis.EventSource = FakeEventSource as unknown as typeof EventSource;
  context.after(() => {
    globalThis.EventSource = originalEventSource;
  });

  const transport = createHostedWebChatTransport("agent-2");
  transport.subscribe(() => {}, () => {}, () => {});
  const first = FakeEventSource.instances[0]!;
  transport.subscribe(() => {}, () => {}, () => {});

  assert.equal(first.closed, true);
  assert.equal(FakeEventSource.instances.length, 2);
  transport.close();
  assert.equal(FakeEventSource.instances[1]!.closed, true);
});

test("hosted reconnect reset lets the shared controller accept a lower fresh baseline", async (context) => {
  const originalEventSource = globalThis.EventSource;
  const originalFetch = globalThis.fetch;
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  const timers: Array<() => void> = [];

  FakeEventSource.instances = [];
  globalThis.EventSource = FakeEventSource as unknown as typeof EventSource;
  globalThis.fetch = (async () => Response.json(appState(42))) as typeof fetch;
  globalThis.setTimeout = ((callback: TimerHandler) => {
    assert.equal(typeof callback, "function");
    timers.push(callback as () => void);
    return timers.length as unknown as ReturnType<typeof setTimeout>;
  }) as typeof setTimeout;
  globalThis.clearTimeout = (() => {}) as typeof clearTimeout;
  context.after(() => {
    globalThis.EventSource = originalEventSource;
    globalThis.fetch = originalFetch;
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  });

  const controller = new ChatProductController(
    createHostedWebChatTransport("agent-controller")
  );
  controller.start();
  await eventually(() => assert.equal(controller.snapshot().state?.rev, 42));

  FakeEventSource.instances[0]!.emit("state", {
    data: JSON.stringify(appState(50)),
  });
  assert.equal(controller.snapshot().state?.rev, 50);

  FakeEventSource.instances[0]!.emit("error", {});
  timers.shift()!();
  FakeEventSource.instances[1]!.emit("state", {
    data: JSON.stringify(appState(1)),
  });

  assert.equal(controller.snapshot().state?.rev, 1);
  assert.equal(controller.snapshot().streamConnected, true);
  controller.stop();
});

class FakeEventSource {
  static instances: FakeEventSource[] = [];

  readonly listeners = new Map<string, Array<(event: unknown) => void>>();
  closed = false;

  constructor(readonly url: string) {
    FakeEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return;
    const callback =
      typeof listener === "function"
        ? (event: unknown) => listener(event as Event)
        : (event: unknown) => listener.handleEvent(event as Event);
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), callback]);
  }

  emit(type: string, event: unknown) {
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }

  close() {
    this.closed = true;
  }
}

function appState(rev: number): AppState {
  return {
    rev,
    identity: { account_id: "account-1", device_id: "web-1" },
    rooms: [],
    topics: [],
    status: "running",
    messages: [],
    profiles: [],
    devices: [],
    typing_members: [],
    flow: {
      notice_busy: false,
      scan_in_flight: false,
      scan_result: "none",
    },
  };
}

async function eventually(assertion: () => void) {
  let error: unknown;
  for (let attempt = 0; attempt < 20; attempt += 1) {
    try {
      assertion();
      return;
    } catch (caught) {
      error = caught;
      await new Promise((resolve) => queueMicrotask(resolve));
    }
  }
  throw error;
}
