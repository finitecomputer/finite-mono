import assert from "node:assert/strict";
import test from "node:test";

import { chatPreviewUrls } from "./chat-preview-urls";

test("chat preview URLs discard trailing Markdown emphasis", () => {
  assert.deepEqual(
    chatPreviewUrls("Published at **https://open-models-explained.finite.chat/**"),
    ["https://open-models-explained.finite.chat/"]
  );
});

test("chat preview URLs prefer Markdown link destinations", () => {
  assert.deepEqual(
    chatPreviewUrls("Open [the generated site](https://example.finite.chat/path?view=full)."),
    ["https://example.finite.chat/path?view=full"]
  );
});

test("chat preview URLs preserve order and de-duplicate Markdown and prose matches", () => {
  assert.deepEqual(
    chatPreviewUrls(
      "[First](https://first.finite.chat/) then https://second.finite.chat/ and https://first.finite.chat/."
    ),
    ["https://first.finite.chat/", "https://second.finite.chat/"]
  );
});
