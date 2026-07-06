import assert from "node:assert/strict";
import test from "node:test";

import {
  clearFiniteBrowserSessionState,
  clearFiniteBrowserStorage,
  finiteBrowserStateKeys,
} from "./browser-session";

class FakeStorage {
  private entries: Map<string, string>;

  constructor(entries: Record<string, string>) {
    this.entries = new Map(Object.entries(entries));
  }

  get length() {
    return this.entries.size;
  }

  key(index: number) {
    return [...this.entries.keys()][index] ?? null;
  }

  removeItem(key: string) {
    this.entries.delete(key);
  }

  has(key: string) {
    return this.entries.has(key);
  }
}

test("finiteBrowserStateKeys only selects finite-owned keys", () => {
  const storage = new FakeStorage({
    "finite.chat.lastMachine": "smoke",
    "finite.dashboard.lastProject": "project-1",
    "next.private": "keep",
    "wos-session": "keep",
  });

  assert.deepEqual(finiteBrowserStateKeys(storage), [
    "finite.chat.lastMachine",
    "finite.dashboard.lastProject",
  ]);
});

test("clearFiniteBrowserStorage removes finite-owned keys and preserves unrelated storage", () => {
  const storage = new FakeStorage({
    "finite.chat.lastMachine": "smoke",
    "finite.dashboard.lastProject": "project-1",
    "next.private": "keep",
    "wos-session": "keep",
  });

  assert.deepEqual(clearFiniteBrowserStorage(storage), [
    "finite.chat.lastMachine",
    "finite.dashboard.lastProject",
  ]);
  assert.equal(storage.has("finite.chat.lastMachine"), false);
  assert.equal(storage.has("finite.dashboard.lastProject"), false);
  assert.equal(storage.has("next.private"), true);
  assert.equal(storage.has("wos-session"), true);
});

test("clearFiniteBrowserSessionState clears local and session finite state", () => {
  const win = {
    localStorage: new FakeStorage({ "finite.local": "1", outside: "2" }),
    sessionStorage: new FakeStorage({ "finite.session": "1", outside: "2" }),
  };

  assert.deepEqual(clearFiniteBrowserSessionState(win), {
    localStorage: ["finite.local"],
    sessionStorage: ["finite.session"],
  });
  assert.equal(win.localStorage.has("finite.local"), false);
  assert.equal(win.localStorage.has("outside"), true);
  assert.equal(win.sessionStorage.has("finite.session"), false);
  assert.equal(win.sessionStorage.has("outside"), true);
});
