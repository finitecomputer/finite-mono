import assert from "node:assert/strict";
import { test } from "node:test";
import { parseBrainIdentities } from "./brain-identity-roster";
import { HostedHermesStatusError } from "./hosted-hermes-status";

const key = "npub1ccz8l9zpa47k6vz9gphftsrumpw80rjt3nhnefat4symjhrsnmjs38mnyd";
const human = { npub: key, role: "admin", kind: "human", name: null, email: "maya@example.test", ownerEmail: null, folders: [{ id: "f1", state: "ready" }] };
const report = { version: 1, brainId: "demo", identities: [human] };

test("resolved humans, agents and unknown keys keep only the roster fields", () => {
  const agent = { ...human, npub: key.replace("ccz8", "ddz8"), role: "member", kind: "agent", name: "Ada", email: null, ownerEmail: "maya@example.test" };
  const unknown = { ...human, npub: key.replace("ccz8", "eez8"), role: "noCurrentRole", kind: null, email: null, folders: [] };
  const rows = parseBrainIdentities({ ...report, identities: [human, agent, { ...unknown, private: "dropped" }] }, "demo");
  assert.deepEqual(rows, [human, agent, unknown]);
});

test("foreign Brains, duplicate or malformed keys, unknown kinds and oversized lists fail closed", () => {
  for (const value of [
    { ...report, brainId: "foreign" }, { ...report, version: 2 }, { ...report, identities: [human, human] },
    { ...report, identities: [{ ...human, npub: "npub1short" }] },
    { ...report, identities: [{ ...human, kind: "verified" }] },
    { ...report, identities: [{ ...human, email: "" }] },
    { ...report, identities: Array(1025).fill(human) },
  ]) assert.throws(() => parseBrainIdentities(value, "demo"), error => error instanceof HostedHermesStatusError && error.kind === "unsupported");
});
