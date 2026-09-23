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


test("native generated media uses existing cards for documents, images and audio", () => {
  const parsed = hermesMessageAttachments('Here are the results\nMEDIA:/data/agent/report.pdf\nMEDIA:"/data/agent/my photo.PNG"\nMEDIA:/data/agent/voice.mp3\nMEDIA:/data/agent/report.pdf');
  assert.equal(parsed.text, "Here are the results");
  assert.deepEqual(parsed.media.map(item => [item.filename, item.kind, item.mime_type]), [
    ["report.pdf", "File", "application/octet-stream"],
    ["my photo.PNG", "Image", "image/png"],
    ["voice.mp3", "File", "audio/mpeg"],
  ]);
});

test("generated-media examples and unsafe paths remain literal text", () => {
  for (const text of ['```text\nMEDIA:/tmp/report.pdf\n```', '~~~\nMEDIA:/tmp/photo.png\n~~~', 'Use `MEDIA:/tmp/report.pdf` as an example', 'Use ``MEDIA:/tmp/report.pdf`` as an example', 'MEDIA:https://other.test/tracker.png', 'MEDIA:relative.pdf', 'MEDIA:/tmp/bad\0name', 'MEDIA:/' + 'x'.repeat(4096)]) {
    assert.deepEqual(hermesMessageAttachments(text), { text, media: [] });
  }
});


test("inline native media preserves the caption and quoted filenames", () => {
  const parsed = hermesMessageAttachments('Listen: MEDIA:/data/agent/voice.mp3\nDownload: MEDIA:`/data/agent/my report.csv`');
  assert.equal(parsed.text, "Listen: \nDownload:");
  assert.deepEqual(parsed.media.map(item => item.filename), ["voice.mp3", "my report.csv"]);
});
