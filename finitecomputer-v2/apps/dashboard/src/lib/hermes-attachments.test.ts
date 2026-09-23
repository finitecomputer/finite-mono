import assert from "node:assert/strict";
import test from "node:test";
import { hermesAttachmentUrl, hermesMessageAttachments } from "./hermes-attachments";

test("native attachment references survive history projection without displaying private paths", () => {
  const parsed = hermesMessageAttachments('Look at this\n@image:`/data/agent/images/my photo.png`\n@file:/data/agent/attachments/report.txt\n@image:`/data/agent/images/my photo.png`');
  assert.equal(parsed.text, "Look at this");
  assert.deepEqual(parsed.media.map(item => [item.filename, item.kind]), [["my photo.png", "Image"], ["report.txt", "File"]]);
  assert.equal(parsed.media[0].mime_type, "image/png");
  const url = new URL(hermesAttachmentUrl("runtime_a", parsed.media[0].attachment_id), "https://finite.test");
  assert.equal(url.pathname, "/api/agents/runtime_a/hermes-file");
  assert.equal(url.searchParams.get("path"), "/data/agent/images/my photo.png");
  assert.equal(url.searchParams.has("token"), false);
});

test("prose, fenced examples and external URLs are never converted into native file requests", () => {
  for (const text of ["Use @file:/tmp/example in the prompt", "```\n@file:/tmp/example\n```", "~~~text\n@image:/tmp/example.png\n~~~", "@image:https://other.test/tracker.png", "@file:relative.txt"]) {
    assert.deepEqual(hermesMessageAttachments(text), { text, media: [] });
  }
  assert.equal(hermesMessageAttachments("@file:/tmp/bad\0name").media.length, 0);
});
