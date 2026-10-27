import assert from "node:assert/strict";
import { test } from "node:test";
import { parseBrainIdentities } from "./brain-identity-roster";
import { HostedHermesStatusError } from "./hosted-hermes-status";

const key = "npub1ccz8l9zpa47k6vz9gphftsrumpw80rjt3nhnefat4symjhrsnmjs38mnyd";
const human = { npub: key, role: "admin", kind: "human", name: null, email: "maya@example.test", ownerEmail: null, folders: [{ id: "f1", state: "ready" }] };
const report = { version: 1, brainId: "demo", identities: [human] };
const extensions = { resolution: null, lifecycle: null, operatorAnnotation: null };
const unnamed = { publicName: null, publicNameStoredAt: null, ...extensions };

test("resolution survives the Runtime boundary and accepts older plugins without inventing a reason", () => {
  const bare = { ...human, kind: null, email: null };
  const resolution = { state: "notShared", reason: "noParticipation" };
  const parsed = parseBrainIdentities({ ...report, identities: [{ ...bare, resolution: { ...resolution, private: "dropped" } }] }, "demo")[0];
  assert.deepEqual(parsed.resolution, resolution);
  assert.equal(parsed.npub, key); assert.equal(parsed.role, "admin");
  assert.deepEqual(parsed.folders, human.folders);
  assert.equal(parseBrainIdentities({ ...report, identities: [bare] }, "demo")[0].resolution, null);
  for (const resolution of [{ state: "invented" }, { state: "unavailable", reason: "private-reason" },
    { state: "resolved", reason: "noParticipation" }, { state: "unknown", reason: "noParticipation" }]) {
    assert.throws(() => parseBrainIdentities({ ...report, identities: [{ ...bare, resolution }] }, "demo"), HostedHermesStatusError);
  }
});

test("resolved humans, agents and unknown keys keep only the roster fields", () => {
  const agent = { ...human, npub: key.replace("ccz8", "ddz8"), role: "member", kind: "agent", name: "Ada", email: null, ownerEmail: "maya@example.test" };
  const unknown = { ...human, npub: key.replace("ccz8", "eez8"), role: "noCurrentRole", kind: null, email: null, folders: [] };
  const rows = parseBrainIdentities({ ...report, identities: [human, agent, { ...unknown, private: "dropped" }] }, "demo");
  // An older plugin sends no public name fields; they parse as absent.
  assert.deepEqual(rows, [{ ...human, ...unnamed }, { ...agent, ...unnamed }, { ...unknown, ...unnamed }]);
});

test("a stored public name passes through with or without its stored time", () => {
  const named = { ...human, kind: null, email: null, publicName: "cara@finite.vip", publicNameStoredAt: "2026-09-17T20:39:30Z" };
  const untimed = { ...named, npub: key.replace("ccz8", "ddz8"), publicNameStoredAt: null };
  assert.deepEqual(parseBrainIdentities({ ...report, identities: [named, untimed] }, "demo"), [{ ...named, ...extensions }, { ...untimed, ...extensions }]);
});

test("stored times keep the server's whole-second form, fractions and offsets", () => {
  const stamps = ["2026-10-05T15:29:27Z", "2024-02-29T00:00:00Z", "2026-09-17T20:39:30.123456789Z", "2026-09-17T20:39:30+05:30", "0001-01-01T00:00:00Z"];
  const rows = stamps.map((publicNameStoredAt, index) => ({ ...human, npub: key.replace("ccz8", `c${"acdefghj"[index]}z8`), publicName: "cara@finite.vip", publicNameStoredAt }));
  assert.deepEqual(parseBrainIdentities({ ...report, identities: rows }, "demo").map(row => row.publicNameStoredAt), stamps);
});

test("impossible stored times fail closed like the plugin's datetime.fromisoformat", () => {
  for (const publicNameStoredAt of ["2026-02-30T20:39:30Z", "2026-09-17T24:00:00Z", "2025-02-29T00:00:00Z", "2026-04-31T00:00:00Z",
    "2026-00-10T00:00:00Z", "2026-09-00T00:00:00Z", "2026-09-17T23:60:00Z", "2026-09-17T23:59:60Z", "0000-01-01T00:00:00Z",
    "2026-09-17T20:39:30+24:00"]) {
    const value = { ...report, identities: [{ ...human, publicName: "cara@finite.vip", publicNameStoredAt }] };
    assert.throws(() => parseBrainIdentities(value, "demo"), error => error instanceof HostedHermesStatusError && error.kind === "unsupported", publicNameStoredAt);
  }
});

test("foreign Brains, duplicate or malformed keys, unknown kinds and oversized lists fail closed", () => {
  for (const value of [
    { ...report, brainId: "foreign" }, { ...report, version: 2 }, { ...report, identities: [human, human] },
    { ...report, identities: [{ ...human, npub: "npub1short" }] },
    { ...report, identities: [{ ...human, kind: "verified" }] },
    { ...report, identities: [{ ...human, email: "" }] },
    { ...report, identities: Array(1025).fill(human) },
    { ...report, identities: [{ ...human, publicName: "" }] },
    { ...report, identities: [{ ...human, publicName: { name: "cara@finite.vip" } }] },
    { ...report, identities: [{ ...human, publicName: "x".repeat(513) }] },
    { ...report, identities: [{ ...human, publicNameStoredAt: "2026-09-17T20:39:30Z" }] },
    { ...report, identities: [{ ...human, publicName: "cara@finite.vip", publicNameStoredAt: "yesterday" }] },
    { ...report, identities: [{ ...human, publicName: "cara@finite.vip", publicNameStoredAt: "2026-13-01T00:00:00Z" }] },
    { ...report, identities: [{ ...human, publicName: "cara@finite.vip", publicNameStoredAt: 1790000000 }] },
  ]) assert.throws(() => parseBrainIdentities(value, "demo"), error => error instanceof HostedHermesStatusError && error.kind === "unsupported");
});

test("dated annotations and current lifecycle remain optional and never supply an Owner", () => {
  const operatorAnnotation = { kind: "historical_identity", label: "Previous example Agent", observedAt: "2026-01-01T00:00:00Z" };
  const historical = { ...human, kind: null, email: null, operatorAnnotation };
  assert.deepEqual(parseBrainIdentities({ ...report, identities: [historical] }, "demo")[0].operatorAnnotation, operatorAnnotation);
  assert.equal(parseBrainIdentities({ ...report, identities: [{ ...human, ownerEmail: "injected@example.test" }] }, "demo")[0].ownerEmail, null);
  const retired = { ...human, kind: "agent", name: "Example", email: null, lifecycle: "retired" };
  assert.equal(parseBrainIdentities({ ...report, identities: [retired] }, "demo")[0].lifecycle, "retired");
  for (const broken of [
    { ...operatorAnnotation, kind: "owner" }, { ...operatorAnnotation, label: " leading" },
    { ...operatorAnnotation, label: "Unicode —" }, { ...operatorAnnotation, label: "x".repeat(121) },
    { ...operatorAnnotation, observedAt: "2026-02-30T00:00:00Z" },
    { ...operatorAnnotation, observedAt: "2026-01-01T00:00:00+00:00" },
    { ...operatorAnnotation, ownerEmail: "injected@example.test" },
  ]) {
    const row = parseBrainIdentities({ ...report, identities: [{ ...historical, operatorAnnotation: broken }] }, "demo")[0];
    assert.equal(row.operatorAnnotation, null);
    assert.equal(row.ownerEmail, null);
    assert.equal(row.name, null);
  }
});
