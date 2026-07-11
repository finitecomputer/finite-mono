import assert from "node:assert/strict";
import test from "node:test";

import type { AppAction, AppState } from "../model";
import { ChatProductController } from "./controller";
import type { ChatProductUpload, ChatTransport } from "./transport";

class FakeTransport implements ChatTransport {
  states: AppState[];
  actions: AppAction[] = [];
  uploads: ChatProductUpload[] = [];
  onState: ((state: AppState) => void) | null = null;
  onError: ((error: Error) => void) | null = null;
  onReset: (() => void) | null = null;

  constructor(...states: AppState[]) {
    this.states = states;
  }

  async load() {
    return this.states[0]!;
  }

  async dispatch(action: AppAction) {
    this.actions.push(action);
    return this.states[Math.min(this.actions.length, this.states.length - 1)]!;
  }

  async upload(upload: ChatProductUpload) {
    this.uploads.push(upload);
    return this.states[this.states.length - 1]!;
  }

  subscribe(
    onState: (state: AppState) => void,
    onError: (error: Error) => void,
    onReset: () => void
  ) {
    this.onState = onState;
    this.onError = onError;
    this.onReset = onReset;
    return () => {
      this.onState = null;
      this.onError = null;
      this.onReset = null;
    };
  }
}

test("controller applies one ordered state stream and ignores stale snapshots", async () => {
  const transport = new FakeTransport(appState(4), appState(6));
  const controller = new ChatProductController(transport);
  const seen: number[] = [];
  controller.listen((view) => {
    if (view.state) seen.push(view.state.rev);
  });

  controller.start();
  await eventually(() => assert.equal(controller.snapshot().state?.rev, 4));
  transport.onState?.(appState(7));
  transport.onState?.(appState(5));

  assert.equal(controller.snapshot().state?.rev, 7);
  assert.deepEqual(seen, [4, 7]);
  controller.stop();
  assert.equal(transport.onState, null);
});

test("daemon generation reset accepts its first lower authoritative revision", async () => {
  const transport = new FakeTransport(appState(42));
  const controller = new ChatProductController(transport);

  controller.start();
  await eventually(() => assert.equal(controller.snapshot().state?.rev, 42));
  transport.onReset?.();
  transport.onState?.(appState(1));

  assert.equal(controller.snapshot().state?.rev, 1);
  assert.equal(controller.snapshot().streamConnected, true);
});

test("actions and uploads use the same typed transport and update shared state", async () => {
  const transport = new FakeTransport(appState(1), appState(2), appState(3));
  const controller = new ChatProductController(transport);

  await controller.dispatch({ SendMessage: { room_id: "room", text: "hello" } });
  assert.deepEqual(transport.actions, [{ SendMessage: { room_id: "room", text: "hello" } }]);
  assert.equal(controller.snapshot().state?.rev, 2);

  await controller.upload({ room_id: "room", caption: "file", files: [] });
  assert.equal(transport.uploads.length, 1);
  assert.equal(controller.snapshot().state?.rev, 3);
});

test("initial load failures become product errors without an unhandled rejection", async () => {
  const transport: ChatTransport = {
    async load() {
      throw new Error("signed-in chat is unavailable");
    },
    async dispatch() {
      throw new Error("not reached");
    },
  };
  const controller = new ChatProductController(transport);

  controller.start();
  await eventually(() => {
    assert.equal(controller.snapshot().loading, false);
    assert.equal(controller.snapshot().error, "signed-in chat is unavailable");
  });
  controller.stop();
});

function appState(rev: number): AppState {
  return {
    rev,
    identity: { account_id: "account", device_id: "device" },
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
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
  }
  throw error;
}
