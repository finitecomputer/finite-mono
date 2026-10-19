import assert from "node:assert/strict";
import { test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { IdentityCell } from "./brain-identity-cell";
import type { BrainIdentity } from "@/lib/brain-identity-roster";

const base: BrainIdentity = {
  npub: "npub1ccz8l9zpa47k6vz9gphftsrumpw80rjt3nhnefat4symjhrsnmjs38mnyd", role: "member", kind: null,
  name: null, email: null, ownerEmail: null, publicName: null, publicNameStoredAt: null, folders: [],
};
const cell = (row: Partial<BrainIdentity>) => renderToStaticMarkup(createElement("table", null,
  createElement("tbody", null, createElement("tr", null, createElement(IdentityCell, { row: { ...base, ...row } })))));
const secondary = (text: string) => `<span class="block text-xs text-muted-foreground">${text}</span>`;
const stored = { publicName: "cara@finite.vip", publicNameStoredAt: "2026-09-17T20:39:30Z" };

test("an unresolved key with a stored public name shows it in place of Unknown", () => {
  const html = cell(stored);
  assert.match(html, new RegExp(`<th scope="row" title="Stored 2026-09-17T20:39:30Z">cara@finite.vip${secondary("Public name")}</th>`));
  assert.doesNotMatch(html, /Unknown/);
  assert.match(cell({ ...stored, publicNameStoredAt: null }), new RegExp(`<th scope="row">cara@finite.vip${secondary("Public name")}</th>`));
});

test("resolved rows ignore the public name and rows with neither stay Unknown", () => {
  assert.match(cell({ ...stored, kind: "agent", name: "Ada" }), new RegExp(`<th scope="row">Ada${secondary("Agent")}</th>`));
  assert.match(cell({ ...stored, kind: "human", email: "maya@example.test" }), /<th scope="row">maya@example.test<\/th>/);
  assert.match(cell({ kind: "human", name: "Maya", email: "maya@example.test" }), new RegExp(`<th scope="row">Maya${secondary("maya@example.test")}</th>`));
  assert.match(cell({}), /<th scope="row"><span class="text-muted-foreground">Unknown<\/span><\/th>/);
  assert.doesNotMatch(cell({ ...stored, kind: "agent", name: "Ada" }), /cara@finite\.vip|Public name|title=/);
});
