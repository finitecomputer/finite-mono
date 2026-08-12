import assert from "node:assert/strict";
import test from "node:test";

import { directHostedImageUrl } from "@/lib/hosted-chat-attachment-url";

test("URL-only Hermes images can render from their HTTPS source", () => {
  const url = "https://v3b.fal.media/files/generated-image.png";
  assert.equal(
    directHostedImageUrl({
      attachment_id: url,
      url,
      kind: "Image",
    }),
    url
  );
});

test("blob-backed and ordinary attachments stay on the authenticated proxy", () => {
  assert.equal(
    directHostedImageUrl({
      attachment_id: "sha256-plaintext-id",
      url: "https://blob.example/encrypted-payload",
      kind: "Image",
    }),
    null
  );
  assert.equal(
    directHostedImageUrl({
      attachment_id: "attachment-1",
      url: null,
      kind: "Image",
    }),
    null
  );
});

test("URL-only non-image media stays on the authenticated proxy", () => {
  const url = "https://media.example/generated-video.mp4";
  assert.equal(
    directHostedImageUrl({
      attachment_id: url,
      url,
      kind: "Video",
    }),
    null
  );
});

test("direct image URLs reject active, credentialed, and insecure remote schemes", () => {
  for (const url of [
    "javascript:alert(1)",
    "data:image/png;base64,AAAA",
    "https://user:secret@example.com/image.png",
    "http://images.example/image.png",
  ]) {
    assert.equal(
      directHostedImageUrl({
        attachment_id: url,
        url,
        kind: "Image",
      }),
      null
    );
  }
});

test("loopback HTTP remains available for local Hermes testing", () => {
  for (const url of [
    "http://localhost:3000/image.png",
    "http://127.0.0.1:3000/image.png",
    "http://[::1]:3000/image.png",
  ]) {
    assert.equal(
      directHostedImageUrl({
        attachment_id: url,
        url,
        kind: "Image",
      }),
      url
    );
  }
});
