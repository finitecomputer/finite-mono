import assert from "node:assert/strict";
import test from "node:test";

import {
  activeAtQuery,
  chatReferencePayloads,
  inlineReferenceToken,
  insertAtReference,
  retainInlineReferences,
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
    {
      text: "compare @pricing.md today",
      cursor: 19,
      reference: { ...reference, token: "@pricing.md" },
    }
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

test("typed reference payloads preserve multiline prose for 1, 3, and 12 references", () => {
  const text = "first line\nsecond line\n\nfinal paragraph";
  for (const count of [1, 3, 12]) {
    const references = Array.from({ length: count }, (_, index): ChatReference => ({
      kind: "file",
      id: `workspace:plans/file-${index}.md`,
      label: `file-${index}.md`,
      detail: `plans/file-${index}.md`,
      path: `plans/file-${index}.md`,
      fingerprint: `sha256:${index}`,
      token: `@file-${index}.md`,
    }));
    const encoded = JSON.stringify({
      text,
      references: chatReferencePayloads(references),
    });
    const decoded = JSON.parse(encoded) as {
      text: string;
      references: ChatReference[];
    };
    assert.equal(decoded.text, text);
    assert.equal(decoded.references.length, count);
    assert.doesNotMatch(decoded.text, /FINITE_CHAT_REFERENCES|File reference:/u);
  }
});

test("colliding labels receive distinct identities and retain by visible token", () => {
  const candidates = ["a", "b", "c"].map((directory): ChatReference => ({
    kind: "file",
    id: `workspace:${directory}/README.md`,
    label: "README.md",
    detail: `${directory}/README.md`,
    path: `${directory}/README.md`,
  }));
  let text = "";
  const selected: ChatReference[] = [];
  for (const candidate of candidates) {
    const queryText = `${text}${text ? " " : ""}@rea`;
    const start = queryText.lastIndexOf("@");
    const inserted = insertAtReference(
      queryText,
      { start, end: queryText.length, query: "rea" },
      candidate,
      selected,
    );
    text = inserted.text;
    selected.push(inserted.reference);
  }

  assert.equal(text, "@README.md @README.md#2 @README.md#3");
  assert.deepEqual(selected.map(inlineReferenceToken), [
    "@README.md",
    "@README.md#2",
    "@README.md#3",
  ]);
  assert.deepEqual(
    retainInlineReferences(text.replace("@README.md#2", ""), selected).map(
      (reference) => reference.id
    ),
    ["workspace:a/README.md", "workspace:c/README.md"],
  );
});
