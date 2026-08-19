import assert from "node:assert/strict";
import test from "node:test";

import { formatBrainDocUrl, parseBrainDocUrl } from "./brain-doc-url";

test("brain doc URLs parse brain-id, folder-id and path", () => {
  assert.deepEqual(parseBrainDocUrl("brain://personal-a/team-notes/roadmap.md"), {
    brainId: "personal-a",
    folderId: "team-notes",
    path: "roadmap.md",
  });
  assert.deepEqual(parseBrainDocUrl("brain://acme/docs/from-envelope.md"), {
    brainId: "acme",
    folderId: "docs",
    path: "from-envelope.md",
  });
});

test("brain doc URLs keep nested paths intact", () => {
  assert.deepEqual(parseBrainDocUrl("brain://b/f/wiki/concepts/example.md"), {
    brainId: "b",
    folderId: "f",
    path: "wiki/concepts/example.md",
  });
});

test("brain doc URLs reject malformed shapes", () => {
  assert.equal(parseBrainDocUrl("https://finite.chat/doc"), null);
  assert.equal(parseBrainDocUrl("brain://only-brain"), null);
  assert.equal(parseBrainDocUrl("brain://brain/folder"), null);
  assert.equal(parseBrainDocUrl("brain://BRAIN/folder/path.md"), null);
  assert.equal(parseBrainDocUrl("brain://brain/folder/"), null);
  assert.equal(parseBrainDocUrl("brain://brain/folder/path with spaces"), null);
});

test("brain doc URLs round-trip through format", () => {
  const doc = parseBrainDocUrl("brain://personal-a/team-notes/roadmap.md");
  assert.ok(doc);
  assert.equal(formatBrainDocUrl(doc), "brain://personal-a/team-notes/roadmap.md");
});
