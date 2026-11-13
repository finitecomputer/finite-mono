import assert from "node:assert/strict";
import { test } from "node:test";
import {
  RosterChangeError, changeMessage, inviteTarget, inviteToBrain, parsePendingInvitations, parseRosterChanges,
  readRemoval, removalMessage, revokeInvitation, startRemoval, waitForRemoval,
} from "./brain-roster-changes";
import { HostedHermesStatusError, type HostedHermesChange } from "./hosted-hermes-status";

const acting = "npub1ccz8l9zpa47k6vz9gphftsrumpw80rjt3nhnefat4symjhrsnmjs38mnyd";
const member = acting.replace("ccz8", "ddz8");
const invitationId = `invitation-${"0".repeat(32)}`;
const jobId = "a".repeat(32);
const pending = { id: invitationId, targetNpub: member, targetName: "mia@finite.vip", expiresAt: "2026-11-02T00:00:00Z", expired: false };

type Sent = { path: string; change: HostedHermesChange };
function agent(responses: Array<{ status: number; body: unknown } | Error>, reads: unknown[] = []) {
  const sent: Sent[] = [];
  const read: string[] = [];
  return {
    sent, read,
    dependencies: {
      send: async (_runtime: string, path: string, change: HostedHermesChange) => {
        sent.push({ path, change });
        const next = responses.shift();
        if (!next) throw new Error("unexpected change");
        if (next instanceof Error) throw next;
        return next;
      },
      read: async (_runtime: string, path: string) => {
        read.push(path);
        const next = reads.shift();
        if (next instanceof Error) throw next;
        return next;
      },
    },
  };
}
const signal = () => new AbortController().signal;

test("the advertised changes need a valid acting key; older agents send none", () => {
  assert.equal(parseRosterChanges({ version: 1 }), null);
  assert.deepEqual(parseRosterChanges({ changes: { actingNpub: acting, actions: ["remove", "invite", "future", "revokeInvitation"] } }),
    { actingNpub: acting, actions: ["invite", "revokeInvitation", "remove"] });
  assert.deepEqual(parseRosterChanges({ changes: { actingNpub: null, actions: ["invite"] } }), { actingNpub: null, actions: [] });
  for (const changes of [{ actingNpub: "npub1short", actions: [] }, { actingNpub: acting, actions: "invite" }, "yes",
    { actingNpub: acting, actions: Array(17).fill("invite") }]) {
    assert.throws(() => parseRosterChanges({ changes }), HostedHermesStatusError);
  }
});

test("invite targets are an npub or a lowercase public name, never flag-shaped", () => {
  assert.equal(inviteTarget(`  ${member}  `), member);
  assert.equal(inviteTarget(" Mia@Finite.VIP "), "mia@finite.vip");
  assert.equal(inviteTarget("_@example.finite.vip"), "_@example.finite.vip");
  for (const value of ["--server", "-mia@finite.vip", "mia@finite.vip --json", "mia@localhost", "mia", "b".repeat(64),
    `${member}q`, `nsec1${"q".repeat(58)}`, `${"a".repeat(65)}@finite.vip`, "mia@finite.vip\nx", ""]) {
    assert.equal(inviteTarget(value), null, value);
  }
});

test("pending invitations parse strictly for this Brain", () => {
  const list = { version: 1, brainId: "acme", invitations: [pending, { ...pending, id: `invitation-${"1".repeat(32)}`, targetNpub: null, targetName: null, expired: true }] };
  assert.equal(parsePendingInvitations(list, "acme").length, 2);
  for (const value of [{ ...list, brainId: "other" }, { ...list, invitations: [pending, pending] },
    { ...list, invitations: [{ ...pending, id: "--id" }] }, { ...list, invitations: [{ ...pending, targetName: "Mia <x>" }] },
    { ...list, invitations: [{ ...pending, expiresAt: "soon" }] }, { ...list, version: 2 }]) {
    assert.throws(() => parsePendingInvitations(value, "acme"), HostedHermesStatusError);
  }
});

test("invite sends one normalized target once and reports who it went to", async () => {
  const fake = agent([{ status: 200, body: { version: 1, invitation: pending } }]);
  assert.deepEqual(await inviteToBrain("rt", "acme", " Mia@finite.vip ", signal(), fake.dependencies), pending);
  assert.deepEqual(fake.sent, [{ path: "api/plugins/finite-brain/invitations/acme", change: { method: "POST", body: { target: "mia@finite.vip" } } }]);
  const unreadable = agent([{ status: 200, body: { version: 1, invitation: { id: "x" } } }]);
  assert.equal(await inviteToBrain("rt", "acme", member, signal(), unreadable.dependencies), null);
  await assert.rejects(inviteToBrain("rt", "acme", "--server", signal(), unreadable.dependencies), (error: unknown) =>
    error instanceof RosterChangeError && error.code === "invalidInput");
  await assert.rejects(inviteToBrain("rt", "--x", member, signal(), unreadable.dependencies));
  assert.equal(unreadable.sent.length, 1);
});

test("every change failure maps to a typed outcome and is never retried", async () => {
  for (const [response, code] of [
    [{ status: 409, body: { error: "busy" } }, "busy"],
    [{ status: 403, body: { error: "notAdmin" } }, "notAdmin"],
    [{ status: 502, body: { error: "failed" } }, "failed"],
    [{ status: 502, body: { error: "unknown" } }, "unknown"],
    [{ status: 504, body: { error: "unknown" } }, "unknown"],
    [{ status: 503, body: { error: "unavailable" } }, "unavailable"],
    [{ status: 422, body: { error: "notPending" } }, "notPending"],
    [{ status: 401, body: null }, "access"],
    [{ status: 404, body: null }, "unavailable"],
    [{ status: 500, body: { error: "private stderr" } }, "unknown"],
    [new HostedHermesStatusError("Hosted access is unavailable for this account or agent.", "access"), "access"],
    [new HostedHermesStatusError("The request timed out. Try again."), "unknown"],
  ] as const) {
    const fake = agent([response]);
    await assert.rejects(revokeInvitation("rt", "acme", invitationId, signal(), fake.dependencies),
      (error: unknown) => error instanceof RosterChangeError && error.code === code, JSON.stringify(response));
    assert.equal(fake.sent.length, 1);
  }
  const fake = agent([]);
  await assert.rejects(revokeInvitation("rt", "acme", "invitation-x", signal(), fake.dependencies));
  assert.equal(fake.sent.length, 0);
});

test("a removal starts once and polls its job until it settles", async () => {
  const running = { version: 1, job: { id: jobId, state: "running" } };
  const fake = agent([{ status: 202, body: running }], [running, new Error("blip"), { version: 1, job: { id: jobId, state: "demotedNotRemoved" } }]);
  assert.equal(await startRemoval("rt", "acme", member, signal(), fake.dependencies), jobId);
  assert.deepEqual(fake.sent[0], { path: "api/plugins/finite-brain/removals/acme", change: { method: "POST", body: { npub: member } } });
  const waits: number[] = [];
  const state = await waitForRemoval("rt", "acme", jobId, signal(), {
    dependencies: fake.dependencies, wait: async (ms: number) => { waits.push(ms); }, intervalMs: 5,
  });
  assert.equal(state, "demotedNotRemoved");
  assert.deepEqual(fake.read, Array(3).fill(`api/plugins/finite-brain/removals/acme/${jobId}`));
  assert.equal(fake.sent.length, 1);
  await assert.rejects(startRemoval("rt", "acme", "npub1x", signal(), fake.dependencies));
  const odd = agent([{ status: 202, body: { version: 1, job: { id: null, state: "running" } } }]);
  await assert.rejects(startRemoval("rt", "acme", member, signal(), odd.dependencies), (error: unknown) =>
    error instanceof RosterChangeError && error.code === "unknown");
});

test("lost, stuck or unreadable jobs read as unknown", async () => {
  const lost = agent([], [{ version: 1, job: { id: jobId, state: "unknown" } }]);
  assert.equal(await waitForRemoval("rt", "acme", jobId, signal(), { dependencies: lost.dependencies, wait: async () => {} }), "unknown");
  const stuck = agent([], Array(4).fill({ version: 1, job: { id: jobId, state: "running" } }));
  assert.equal(await waitForRemoval("rt", "acme", jobId, signal(), { dependencies: stuck.dependencies, wait: async () => {}, maxPolls: 4 }), "unknown");
  const failing = agent([], Array(5).fill(new Error("down")));
  assert.equal(await waitForRemoval("rt", "acme", jobId, signal(), { dependencies: failing.dependencies, wait: async () => {} }), "unknown");
  assert.equal(await readRemoval("rt", "acme", "../x", signal(), failing.dependencies), "unknown");
});

test("outcome copy is plain, names the person and has no em dashes", () => {
  const all = [
    ...(["busy", "notAdmin", "invalidInput", "notPending", "notRemovable", "ownKey", "failed", "unknown", "unavailable", "access"] as const)
      .flatMap(code => (["invite", "revoke", "remove"] as const).map(action => changeMessage(code, "Ada", action, "Mia"))),
    ...(["removed", "demotedNotRemoved", "failed", "unknown"] as const).map(state => removalMessage(state, "Ada", "Mia")),
  ];
  for (const text of all) assert.doesNotMatch(text, /\u2014/);
  assert.equal(changeMessage("failed", "Ada", "remove", "Mia"), "Could not remove Mia. Ask Ada in Chat for details.");
  assert.equal(changeMessage("invalidInput", "Ada", "revoke", "Mia"), "That request was not accepted. Refresh and try again.");
  assert.equal(removalMessage("demotedNotRemoved", "Ada", "Mia"), "Mia is now a Member and was not removed. Ask Ada in Chat for details.");
  assert.equal(removalMessage("unknown", "Ada", "Mia"), "Could not confirm whether Mia was removed. Refresh to check.");
});
