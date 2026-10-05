import assert from "node:assert/strict";
import { test } from "node:test";

import { type AccountAuthContext } from "./dashboard-auth";
import {
  OBSERVATION_CREDENTIAL_HEADER,
  OBSERVATION_PATH,
  acceptedBrainInvitation,
  appliedBrainApproval,
  brainObservationConfig,
  observeHostedBrainInvitationAcceptance,
} from "./brain-identity-observation";

const HEX = "ab".repeat(32);
const NPUB = "npub1synthetichostedkey";
const CONFIG = { coreUrl: "http://127.0.0.1:4202", credential: "observation-token" };
const ACCOUNT: AccountAuthContext = {
  email: "dana@acme.example",
  workosUserId: "user_workos",
  emailVerified: true,
  accessToken: "workos-access-token",
  source: "workos",
};
const ACCEPTED = { brainId: "brain_alpha", status: "accepted", userId: NPUB };

type Call = { url: string; init: RequestInit };

function recorder(responses: Array<Response | Error>) {
  const calls: Call[] = [];
  const fetcher = (async (url: string, init: RequestInit) => {
    calls.push({ url, init });
    const next = responses.shift();
    if (!next || next instanceof Error) throw next ?? new Error("no response");
    return next;
  }) as unknown as typeof fetch;
  return { calls, fetcher };
}

function deps(fetcher: typeof fetch, identify: () => Promise<unknown> = async () => ({
  publicKeyHex: HEX,
  npub: NPUB,
})) {
  return { identifyMember: identify, fetch: fetcher, now: () => new Date("2026-10-04T00:00:00Z") };
}

test("configuration needs both the Core URL and the credential", () => {
  assert.equal(brainObservationConfig({}), null);
  assert.equal(brainObservationConfig({ FC_CORE_BRAIN_IDENTITY_URL: "http://127.0.0.1:4202" }), null);
  assert.equal(
    brainObservationConfig({
      FC_CORE_BRAIN_IDENTITY_URL: "http://127.0.0.1:4202/path",
      FC_CORE_BRAIN_OBSERVATION_TOKEN: "t",
    }),
    null
  );
  assert.deepEqual(
    brainObservationConfig({
      FC_CORE_BRAIN_IDENTITY_URL: "http://127.0.0.1:4202/",
      FC_CORE_BRAIN_OBSERVATION_TOKEN: "t",
    }),
    { coreUrl: "http://127.0.0.1:4202", credential: "t" }
  );
});

test("acceptance must name this exact hosted key and an accepted status", () => {
  assert.deepEqual(acceptedBrainInvitation(ACCEPTED, NPUB), { brainId: "brain_alpha" });
  assert.deepEqual(
    acceptedBrainInvitation({ brainId: "b", status: "accepted", claimedByNpub: NPUB }, NPUB),
    { brainId: "b" }
  );
  assert.equal(acceptedBrainInvitation({ ...ACCEPTED, status: "pending" }, NPUB), null);
  assert.equal(acceptedBrainInvitation({ ...ACCEPTED, userId: "npub1other" }, NPUB), null);
  assert.equal(acceptedBrainInvitation({ ...ACCEPTED, brainId: "../x" }, NPUB), null);
  assert.equal(acceptedBrainInvitation({ status: "ok" }, NPUB), null);
  assert.equal(acceptedBrainInvitation(null, NPUB), null);
});

test("a proven acceptance posts one observation with both credentials", async () => {
  const { calls, fetcher } = recorder([
    Response.json({ version: "v", operationId: "x", outcome: "recorded" }),
  ]);
  const result = await observeHostedBrainInvitationAcceptance(
    CONFIG,
    ACCOUNT,
    "https://brain.finite.computer",
    ACCEPTED,
    deps(fetcher)
  );
  assert.deepEqual(result, { outcome: "recorded" });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, `${CONFIG.coreUrl}${OBSERVATION_PATH}`);
  const headers = calls[0].init.headers as Record<string, string>;
  assert.equal(headers.authorization, "Bearer workos-access-token");
  assert.equal(headers[OBSERVATION_CREDENTIAL_HEADER], "observation-token");
  const body = JSON.parse(String(calls[0].init.body));
  assert.equal(body.brainServer, "https://brain.finite.computer");
  assert.equal(body.brainId, "brain_alpha");
  assert.equal(body.actionKind, "humanHostedAction");
  assert.equal(body.observedHumanPublicKeyHex, HEX);
  assert.equal(body.participatingPublicKeyHex, HEX);
  // No email, account id or browser-supplied field is sent.
  assert.equal(JSON.stringify(body).includes("dana@acme.example"), false);
  assert.equal(JSON.stringify(body).includes("user_workos"), false);
});

test("a lost response is retried once with the same operation id", async () => {
  const { calls, fetcher } = recorder([
    new Error("socket hang up"),
    Response.json({ outcome: "recorded" }),
  ]);
  const result = await observeHostedBrainInvitationAcceptance(
    CONFIG,
    ACCOUNT,
    "https://brain.finite.computer",
    ACCEPTED,
    deps(fetcher)
  );
  assert.deepEqual(result, { outcome: "recorded" });
  assert.equal(calls.length, 2);
  assert.equal(calls[0].init.body, calls[1].init.body);
});

test("Core outages, refusals and old Core never throw", async () => {
  for (const responses of [
    [new Error("down"), new Error("down")],
    [new Response("", { status: 503 }), new Response("", { status: 503 })],
    [new Response("", { status: 404 })],
    [new Response("", { status: 409 })],
    [new Response("not json", { status: 200 })],
  ]) {
    const { fetcher } = recorder([...responses]);
    const result = await observeHostedBrainInvitationAcceptance(
      CONFIG,
      ACCOUNT,
      "https://brain.finite.computer",
      ACCEPTED,
      deps(fetcher)
    );
    assert.ok("skipped" in result, JSON.stringify(result));
  }
});

test("missing Device identity, unproven acceptance or no config send nothing", async () => {
  const cases: Array<[Parameters<typeof observeHostedBrainInvitationAcceptance>, string]> = [];
  const { calls, fetcher } = recorder([]);
  cases.push([
    [
      CONFIG,
      ACCOUNT,
      "https://brain.finite.computer",
      ACCEPTED,
      deps(fetcher, async () => {
        throw new Error("Brain identity setup required");
      }),
    ],
    "no hosted identity",
  ]);
  cases.push([
    [CONFIG, ACCOUNT, "https://brain.finite.computer", ACCEPTED, deps(fetcher, async () => ({}))],
    "no hosted identity",
  ]);
  cases.push([
    [
      CONFIG,
      ACCOUNT,
      "https://brain.finite.computer",
      { ...ACCEPTED, userId: "npub1other" },
      deps(fetcher),
    ],
    "action not proven",
  ]);
  cases.push([[null, ACCOUNT, "https://brain.finite.computer", ACCEPTED, deps(fetcher)], "not configured"]);
  cases.push([
    [CONFIG, { ...ACCOUNT, accessToken: undefined }, "https://brain.finite.computer", ACCEPTED, deps(fetcher)],
    "no account session",
  ]);
  for (const [args, reason] of cases) {
    assert.deepEqual(await observeHostedBrainInvitationAcceptance(...args), { skipped: reason });
  }
  assert.equal(calls.length, 0);
});

test("an applied delegation-grant approval proves its signed Brain", async () => {
  const applied = { status: "applied", action: "delegation-grant", approvalEventId: "e", result: {} };
  assert.deepEqual(appliedBrainApproval(applied, "brain_alpha"), { brainId: "brain_alpha" });
  assert.equal(appliedBrainApproval({ ...applied, status: "pending" }, "brain_alpha"), null);
  assert.equal(appliedBrainApproval({ ...applied, action: "other" }, "brain_alpha"), null);
  assert.equal(appliedBrainApproval(applied, "../brain"), null);
  assert.equal(appliedBrainApproval(null, "brain_alpha"), null);
});
