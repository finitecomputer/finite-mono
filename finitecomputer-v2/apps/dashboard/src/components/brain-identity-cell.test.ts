import assert from "node:assert/strict";
import { test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { IdentityCell, identityLabel } from "./brain-identity-cell";
import type { BrainIdentity } from "@/lib/brain-identity-roster";

const base: BrainIdentity = {
  npub: "npub1ccz8l9zpa47k6vz9gphftsrumpw80rjt3nhnefat4symjhrsnmjs38mnyd", role: "member", kind: null,
  name: null, email: null, ownerEmail: null, publicName: null, publicNameStoredAt: null, lifecycle: null, operatorAnnotation: null, folders: [],
};
const cell = (row: Partial<BrainIdentity>, alias?: string) => renderToStaticMarkup(createElement("table", null,
  createElement("tbody", null, createElement("tr", null, createElement(IdentityCell, { row: { ...base, ...row }, alias })))));
const secondary = (text: string) => `<span class="block text-xs text-muted-foreground">${text}</span>`;
const stored = { publicName: "cara@finite.vip", publicNameStoredAt: "2026-09-17T20:39:30Z" };

test("unresolved rows explain the server state without turning the reason or a private alias into an identity", () => {
  const cases: [NonNullable<BrainIdentity["resolution"]>, string][] = [
    [{ state: "notShared", reason: "noParticipation" }, "This key has not acted in this Brain yet."],
    [{ state: "notShared", reason: null }, "Identity details are not available to this Brain."],
    [{ state: "unknown", reason: null }, "No supported identity details are available."],
    [{ state: "ambiguous", reason: null }, "Identity records conflict."],
    [{ state: "unavailable", reason: null }, "Identity lookup is unavailable. Try again later."],
  ];
  for (const [resolution, message] of cases) {
    assert(cell({ resolution }).includes(message));
    const aliased = cell({ resolution }, "My colleague");
    assert(aliased.includes("My colleague") && aliased.includes("Your alias") && aliased.includes(message));
    // Confirmation callers still have no identity name and must use the key.
    assert.equal(identityLabel({ ...base, resolution }).primary, null);
    assert.equal(identityLabel({ ...base, resolution }).detail, null);
    assert(!cell({ ...stored, resolution }).includes(message));
  }
});

test("an unresolved key with a stored public name shows it in place of Unknown", () => {
  const html = cell(stored);
  assert.match(html, new RegExp(`<th scope="row" title="Stored 2026-09-17T20:39:30Z">cara@finite.vip${secondary("Public name")}</th>`));
  assert.doesNotMatch(html, /Unknown/);
  assert.match(cell({ ...stored, publicNameStoredAt: null }), new RegExp(`<th scope="row">cara@finite.vip${secondary("Public name")}</th>`));
});

test("resolved rows ignore the public name and older rows with neither explain that details are unavailable", () => {
  assert.match(cell({ ...stored, kind: "agent", name: "Ada" }), new RegExp(`<th scope="row">Ada${secondary("Agent")}</th>`));
  assert.match(cell({ ...stored, kind: "human", email: "maya@example.test" }), /<th scope="row">maya@example.test<\/th>/);
  assert.match(cell({ kind: "human", name: "Maya", email: "maya@example.test" }), new RegExp(`<th scope="row">Maya${secondary("maya@example.test")}</th>`));
  assert.match(cell({}), /<th scope="row"><span class="text-muted-foreground">Identity details are unavailable.<\/span><\/th>/);
  assert.doesNotMatch(cell({ ...stored, kind: "agent", name: "Ada" }), /cara@finite\.vip|Public name|title=/);
});

test("annotations render as dated text and lifecycle qualifiers stay in the existing Identity cell", () => {
  const annotation = { kind: "historical_identity" as const, label: "Previous <Agent>", observedAt: "2026-01-01T00:00:00Z" };
  const html = cell({ ...stored, operatorAnnotation: annotation });
  assert.match(html, /Previous &lt;Agent&gt;/);
  assert.match(html, /Historical identity · Evidence 2026-01-01/);
  assert.doesNotMatch(html, /Public name|Owner/);
  assert.match(cell({ kind: "agent", name: "Example", lifecycle: "retired" }), /Agent \(retired\)/);
  assert.match(cell({ kind: "agent", name: "Example", lifecycle: "offboarding" }), /Agent \(offboarding\)/);
  assert.doesNotMatch(cell({ kind: "human", email: "example@example.test", operatorAnnotation: annotation }), /Previous/);
});


test("private aliases preserve authoritative identity and every scoped fallback", () => {
  const cases: [Partial<BrainIdentity>, string[]][] = [
    [{ kind: "human", name: "Maya", email: "maya@example.test" }, ["Maya", "maya@example.test"]],
    [{ kind: "human", email: "maya@example.test" }, ["maya@example.test"]],
    [{ kind: "agent", name: "Example", lifecycle: "retired" }, ["Example", "Agent (retired)"]],
    [{ operatorAnnotation: { kind: "invitation_key", label: "Invitation", observedAt: "2026-01-01T00:00:00Z" } }, ["Invitation", "Invitation key · Evidence 2026-01-01"]],
    [stored, [stored.publicName, "Public name"]],
    [{}, ["Identity details are unavailable."]],
  ];
  for (const [row, labels] of cases) {
    const html = cell(row, "My colleague");
    assert(html.includes("My colleague"));
    assert(html.includes("Your alias"));
    for (const label of labels) assert(html.includes(label), `${label} missing from ${html}`);
  }
});
