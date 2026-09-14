import assert from "node:assert/strict";
import test from "node:test";

import {
  copyChatResponse,
  shareChatResponse,
} from "@/lib/chat-response-actions";

test("copy writes the response text and reports visible success", async () => {
  const writes: string[] = [];

  assert.equal(
    await copyChatResponse(
      { clipboard: { writeText: async (text) => void writes.push(text) } },
      "A useful answer."
    ),
    "copied"
  );
  assert.deepEqual(writes, ["A useful answer."]);
});

test("share uses the native share sheet when it is available", async () => {
  const shares: Array<{ text: string; title: string }> = [];
  const payload = { text: "A useful answer.", title: "Launch planning" };

  assert.equal(
    await shareChatResponse(
      { share: async (data) => void shares.push(data) },
      payload
    ),
    "shared"
  );
  assert.deepEqual(shares, [payload]);
});

test("share falls back to the clipboard when no share sheet is available", async () => {
  const writes: string[] = [];

  assert.equal(
    await shareChatResponse(
      { clipboard: { writeText: async (text) => void writes.push(text) } },
      { text: "Portable answer.", title: "Research" }
    ),
    "copied"
  );
  assert.deepEqual(writes, ["Portable answer."]);
});

test("cancelling the native share sheet does not copy or report an error", async () => {
  const writes: string[] = [];
  const cancellation = new Error("cancelled");
  cancellation.name = "AbortError";

  assert.equal(
    await shareChatResponse(
      {
        clipboard: { writeText: async (text) => void writes.push(text) },
        share: async () => { throw cancellation; },
      },
      { text: "Private answer.", title: "Private chat" }
    ),
    "cancelled"
  );
  assert.deepEqual(writes, []);
});

test("copy failures are contained and surfaced as a result", async () => {
  assert.equal(await copyChatResponse({}, "Answer"), "failed");
  assert.equal(
    await copyChatResponse(
      { clipboard: { writeText: async () => { throw new Error("denied"); } } },
      "Answer"
    ),
    "failed"
  );
});
