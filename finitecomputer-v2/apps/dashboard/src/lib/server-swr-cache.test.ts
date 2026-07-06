import assert from "node:assert/strict";
import test from "node:test";

import {
  invalidateServerSwrCache,
  readThroughServerSwr,
} from "./server-swr-cache";

test("server SWR cache returns stale data while refreshing in the background", async () => {
  invalidateServerSwrCache("test-swr:");
  let calls = 0;
  const values = ["first", "second"];

  const first = await readThroughServerSwr(
    "test-swr:item",
    { freshMs: 1_000, staleMs: 1_000 },
    async () => values[calls++]!
  );
  assert.equal(first, "first");

  const stale = await readThroughServerSwr(
    "test-swr:item",
    { freshMs: -1, staleMs: 1_000 },
    async () => values[calls++]!
  );
  assert.equal(stale, "first");

  await new Promise((resolve) => setTimeout(resolve, 0));

  const refreshed = await readThroughServerSwr(
    "test-swr:item",
    { freshMs: 1_000, staleMs: 1_000, nowMs: Date.now() },
    async () => {
      throw new Error("should not reload fresh value");
    }
  );
  assert.equal(refreshed, "second");
  assert.equal(calls, 2);
});

test("server SWR cache can invalidate by prefix", async () => {
  invalidateServerSwrCache("test-swr:");
  let calls = 0;

  await readThroughServerSwr(
    "test-swr:item",
    { freshMs: 1_000, staleMs: 1_000 },
    async () => ++calls
  );
  invalidateServerSwrCache("test-swr:");
  const value = await readThroughServerSwr(
    "test-swr:item",
    { freshMs: 1_000, staleMs: 1_000 },
    async () => ++calls
  );

  assert.equal(value, 2);
});
