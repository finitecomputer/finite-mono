import assert from "node:assert/strict";
import test from "node:test";

import {
  activeAtQuery,
  inlineReferenceToken,
  insertAtReference,
  parseChatReferences,
  retainInlineReferences,
  serializeChatReferences,
  type ChatReference,
} from "./chat-references";

test("@ search opens at a boundary but not in email or code", () => {
  assert.deepEqual(activeAtQuery("compare @pric", 13), {
    start: 8,
    end: 13,
    query: "pric",
  });
  assert.equal(activeAtQuery("person@example.com", 18), null);
  assert.equal(activeAtQuery("run `echo @price`", 16), null);
  assert.equal(activeAtQuery("```\n@price", 10), null);
});

test("selecting replaces the query with a minimal inline reference", () => {
  const reference: ChatReference = {
    kind: "file",
    id: "workspace:plans/pricing.md",
    label: "pricing.md",
    detail: "plans/pricing.md",
  };
  assert.deepEqual(
    insertAtReference(
      "compare @pric today",
      { start: 8, end: 13, query: "pric" },
      reference,
    ),
    { text: "compare @pricing.md today", cursor: 19 }
  );
  assert.equal(inlineReferenceToken(reference), "@pricing.md");
});

test("selected inline references do not reopen search and disappear with their text", () => {
  const reference: ChatReference = {
    kind: "skill",
    id: "skill:strategy-review",
    label: "strategy-review",
    detail: "Review a product strategy.",
  };
  assert.equal(
    activeAtQuery("Use @strategy-review next", 26, [reference]),
    null,
  );
  assert.deepEqual(retainInlineReferences("Use @strategy-review next", [reference]), [reference]);
  assert.deepEqual(retainInlineReferences("Use strategy-review next", [reference]), []);
});

test("reference serialization stays readable to the agent and renders as typed data", () => {
  const references: ChatReference[] = [
    {
      kind: "file",
      id: "workspace:plans/pricing.md",
      label: "pricing.md",
      detail: "plans/pricing.md",
      path: "plans/pricing.md",
      fingerprint: "mtime-size:1:2",
    },
    {
      kind: "skill",
      id: "skill:strategy-review",
      label: "strategy-review",
      detail: "Review a product strategy.",
    },
  ];
  const serialized = serializeChatReferences("Compare these.", references);
  assert.match(serialized, /Open and use this Agent workspace Markdown file/u);
  assert.match(serialized, /Load and follow this skill/u);
  assert.deepEqual(parseChatReferences(serialized), {
    text: "Compare these.",
    references,
  });
});
