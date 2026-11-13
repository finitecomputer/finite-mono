import assert from "node:assert/strict";
import { test } from "node:test";
import { parseBrainInventory } from "./agent-product-inventory";
import {
  createdPersonalBrainId,
  needsPersonalBrainSetup,
  parseOwnerNpub,
  parsePersonalAgentConsent,
  brainRefusal,
  parsePersonalBrainSetupRequest,
  personalBrainConflictMessage,
  personalBrainCreateBody,
} from "./personal-brain-setup";

const owner = `npub1${"q".repeat(58)}`;
const agent = `npub1${"p".repeat(58)}`;
const consent = {
  id: "a".repeat(64),
  pubkey: "b".repeat(64),
  created_at: 1_790_000_000,
  kind: 30_078,
  tags: [["d", "personal-0123456789abcdef"], ["p", "c".repeat(64)]],
  content: '{"brainId":"personal-0123456789abcdef"}',
  sig: "d".repeat(128),
};
const response = { version: 1, agentNpub: agent, ownerNpub: owner, brainId: "personal-0123456789abcdef", consent };
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));

test("Agent consent must name the requested owner, a different Agent and a personal Brain id", () => {
  assert.deepEqual(parsePersonalAgentConsent(response, owner), {
    agentNpub: agent, brainId: "personal-0123456789abcdef", consent,
  });
  const refusals: [string, (data: Record<string, unknown>) => void][] = [
    ["other owner", data => { data.ownerNpub = `npub1${"z".repeat(58)}`; }],
    ["self consent", data => { data.agentNpub = owner; }],
    ["malformed agent", data => { data.agentNpub = "npub1agent"; }],
    ["uppercase agent", data => { data.agentNpub = agent.toUpperCase(); }],
    ["organization id", data => { data.brainId = "acme"; }],
    ["uppercase id", data => { data.brainId = "personal-0123456789ABCDEF"; }],
    ["future version", data => { data.version = 2; }],
    ["CLI version", data => { data.version = "finite-brain-personal-agent-consent-v1"; }],
    ["missing consent", data => { delete data.consent; }],
    ["unsigned", data => { delete (data.consent as Record<string, unknown>).sig; }],
    ["short id", data => { (data.consent as Record<string, unknown>).id = "a".repeat(63); }],
    ["string time", data => { (data.consent as Record<string, unknown>).created_at = "1790000000"; }],
    ["fractional kind", data => { (data.consent as Record<string, unknown>).kind = 1.5; }],
    ["flat tags", data => { (data.consent as Record<string, unknown>).tags = ["d", "x"]; }],
    ["empty content", data => { (data.consent as Record<string, unknown>).content = ""; }],
  ];
  for (const [name, mutate] of refusals) {
    const data = clone(response) as Record<string, unknown>;
    mutate(data);
    assert.equal(parsePersonalAgentConsent(data, owner), null, name);
  }
  assert.equal(parsePersonalAgentConsent(response, "npub1short"), null);
  assert.equal(parsePersonalAgentConsent([response], owner), null);
});

test("Setup requests keep only the signed event fields and build the owner-signed creation body", () => {
  const extra = { ...clone(response), consent: { ...consent, note: "dropped" } };
  const setup = parsePersonalBrainSetupRequest(extra);
  assert(setup);
  assert.deepEqual(setup.consent, consent);
  assert.deepEqual(JSON.parse(personalBrainCreateBody(setup)), {
    brainId: "personal-0123456789abcdef",
    kind: "personal",
    name: "Personal Brain",
    personalAgentNpub: agent,
    personalAgentConsent: consent,
  });
  assert.equal(parsePersonalBrainSetupRequest({ ...response, brainId: "../personal" }), null);
  assert.equal(parsePersonalBrainSetupRequest({ ...response, consent: [consent] }), null);
  assert.equal(parsePersonalBrainSetupRequest(null), null);
  assert.equal(parseOwnerNpub({ ownerNpub: owner }), owner);
  assert.equal(parseOwnerNpub({ ownerNpub: "npub1short" }), null);
  assert.equal(createdPersonalBrainId({ brainId: "personal-0123456789abcdef" }, "fallback"), "personal-0123456789abcdef");
  assert.equal(createdPersonalBrainId({ status: "ok" }, "personal-fedcba9876543210"), "personal-fedcba9876543210");
  assert.equal(createdPersonalBrainId({ brainId: "../x" }, "personal-fedcba9876543210"), "personal-fedcba9876543210");
});

test("Setup is offered only for a loaded inventory without this Agent's own Personal Brain", () => {
  const row = (role: string, kind = "personal", folders: unknown = null) => ({ id: `${kind}-${role}`, name: role, kind, role, folders });
  const inventory = (...brains: unknown[]) => parseBrainInventory({ version: 1, brains }).brains;
  assert.equal(needsPersonalBrainSetup(null), false);
  assert.equal(needsPersonalBrainSetup(undefined), false);
  assert.equal(needsPersonalBrainSetup(inventory()), true);
  assert.equal(needsPersonalBrainSetup(inventory(row("member", "organization", []))), true);
  assert.equal(needsPersonalBrainSetup(inventory(row("member"), row("guest"))), true);
  assert.equal(needsPersonalBrainSetup(inventory(row("invited"))), true);
  assert.equal(needsPersonalBrainSetup(inventory(row("personal_agent"))), false);
  assert.equal(needsPersonalBrainSetup(inventory(row("owner"), row("member", "organization", []))), false);
  assert.equal(needsPersonalBrainSetup(inventory(row("owner", "organization", []))), true);
});

test("Brain conflicts read as plain sentences", () => {
  assert.equal(
    personalBrainConflictMessage("agent is already the personal agent of another personal brain"),
    "This agent is already the Personal Agent of another Personal Brain.",
  );
  assert.equal(
    personalBrainConflictMessage("personal brain already has a different personal agent"),
    "Your Personal Brain already has a different Personal Agent.",
  );
  assert.equal(personalBrainConflictMessage("user already has a personal brain"), "You already have a Personal Brain.");
  assert.equal(
    personalBrainConflictMessage("duplicate id for brain_id: personal-0123456789abcdef"),
    "Your Personal Brain could not be set up. Refresh and try again.",
  );
});

test("Only a server failure invites a retry", () => {
  assert.equal(brainRefusal(400), "Brain refused this Personal Brain setup.");
  assert.equal(brainRefusal(403), "Brain refused this Personal Brain setup.");
  assert.equal(brainRefusal(503), "Brain could not set up your Personal Brain. Try again.");
});
