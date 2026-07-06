import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_INVITE_WAIT_TIMEOUT_MS,
  agentInviteWaitStampRedirectPath,
  fetchAgentInvite,
  isFiniteJoinUrl,
  parseAgentInviteResponse,
  parseAgentInviteWaitStartedAt,
  resolveAgentInviteDisplay,
  truncateInviteUrl,
  truncateNpub,
} from "./agent-invite";

const INVITE_URL = "finite://join?invite=abc123&relay=wss%3A%2F%2Frelay.example";
const STATUS_URL = "https://runtime.example.com/invite";
const READY_AT_MS = 1_750_000_000_000;

test("isFiniteJoinUrl accepts only finite://join URIs", () => {
  assert.equal(isFiniteJoinUrl(INVITE_URL), true);
  assert.equal(isFiniteJoinUrl("finite://join"), true);
  assert.equal(isFiniteJoinUrl("finite://other?x=1"), false);
  assert.equal(isFiniteJoinUrl("https://runtime.example.com/invite"), false);
  assert.equal(isFiniteJoinUrl("javascript:alert(1)"), false);
  assert.equal(isFiniteJoinUrl("not a url"), false);
  assert.equal(isFiniteJoinUrl(""), false);
  assert.equal(isFiniteJoinUrl(null), false);
  assert.equal(isFiniteJoinUrl(42), false);
});

test("parseAgentInviteResponse maps the invite status payload", () => {
  assert.deepEqual(
    parseAgentInviteResponse({
      ready: true,
      room_id: "room-1",
      invite_id: "inv-1",
      url: INVITE_URL,
    }),
    { state: "ready", inviteUrl: INVITE_URL, roomId: "room-1" }
  );

  // Missing or blank room ids read as null.
  assert.deepEqual(parseAgentInviteResponse({ ready: true, url: INVITE_URL }), {
    state: "ready",
    inviteUrl: INVITE_URL,
    roomId: null,
  });
  assert.deepEqual(
    parseAgentInviteResponse({ ready: true, url: INVITE_URL, room_id: "  " }),
    { state: "ready", inviteUrl: INVITE_URL, roomId: null }
  );

  assert.deepEqual(parseAgentInviteResponse({ ready: false }), { state: "pending" });
  assert.deepEqual(parseAgentInviteResponse({ ready: false, error: "   " }), {
    state: "pending",
  });
  assert.deepEqual(
    parseAgentInviteResponse({ ready: false, error: "invite bridge crashed" }),
    { state: "error", message: "invite bridge crashed" }
  );
});

test("parseAgentInviteResponse maps a consumed invite to the paired state", () => {
  assert.deepEqual(
    parseAgentInviteResponse({
      ready: true,
      paired: true,
      agent_npub: "npub1agentexamplekey",
      account_id: "acct-1",
      room_id: "room-1",
      invite_state: "consumed",
    }),
    { state: "paired", roomId: "room-1", agentNpub: "npub1agentexamplekey" }
  );

  // Missing or blank room ids and npubs read as null.
  assert.deepEqual(parseAgentInviteResponse({ ready: true, paired: true }), {
    state: "paired",
    roomId: null,
    agentNpub: null,
  });
  assert.deepEqual(
    parseAgentInviteResponse({ ready: true, paired: true, agent_npub: "  ", room_id: "" }),
    { state: "paired", roomId: null, agentNpub: null }
  );

  // Only a literal `true` counts as paired.
  assert.deepEqual(parseAgentInviteResponse({ ready: false, paired: "yes" }), {
    state: "pending",
  });
});

test("parseAgentInviteResponse lets paired take precedence over a lingering URL", () => {
  assert.deepEqual(
    parseAgentInviteResponse({
      ready: true,
      paired: true,
      url: INVITE_URL,
      room_id: "room-1",
      invite_state: "consumed",
    }),
    { state: "paired", roomId: "room-1", agentNpub: null }
  );
});

test("parseAgentInviteResponse maps a missing invite session to a distinct error", () => {
  const result = parseAgentInviteResponse({
    ready: true,
    paired: false,
    invite_state: "not_found",
  });
  assert.equal(result.state, "error");
  assert.ok(result.state === "error" && /invite session/i.test(result.message));
  assert.ok(result.state === "error" && /new invite/i.test(result.message));
});

test("parseAgentInviteResponse keeps unknown-probe responses on the ready path", () => {
  // Status probe unreachable: URL still present, no expiry — render the QR.
  assert.deepEqual(
    parseAgentInviteResponse({
      ready: true,
      paired: false,
      url: INVITE_URL,
      invite_state: "unknown",
    }),
    { state: "ready", inviteUrl: INVITE_URL, roomId: null }
  );
});

test("parseAgentInviteResponse never yields a ready state without a valid join URL", () => {
  for (const url of [undefined, null, "", "https://evil.example", "not a url", 7]) {
    const result = parseAgentInviteResponse({ ready: true, url });
    assert.equal(result.state, "error", JSON.stringify(url));
  }
});

test("parseAgentInviteResponse treats malformed payloads as pending", () => {
  for (const payload of [null, undefined, "ready", 12, [], true]) {
    assert.deepEqual(parseAgentInviteResponse(payload), { state: "pending" }, String(payload));
  }
});

test("fetchAgentInvite returns the parsed status for a ready invite", async () => {
  const previousFetch = globalThis.fetch;
  const requested: string[] = [];
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    requested.push(String(input));
    return new Response(
      JSON.stringify({ ready: true, room_id: "room-1", url: INVITE_URL }),
      { status: 200 }
    );
  }) as typeof fetch;

  try {
    assert.deepEqual(await fetchAgentInvite(STATUS_URL), {
      state: "ready",
      inviteUrl: INVITE_URL,
      roomId: "room-1",
    });
    assert.deepEqual(requested, [STATUS_URL]);
  } finally {
    globalThis.fetch = previousFetch;
  }
});

test("fetchAgentInvite treats non-OK responses and malformed JSON as pending", async () => {
  const previousFetch = globalThis.fetch;
  const responses = [
    new Response("{}", { status: 404 }),
    new Response("<html>bad gateway</html>", { status: 200 }),
  ];
  globalThis.fetch = (async () => {
    const response = responses.shift();
    assert.ok(response);
    return response;
  }) as typeof fetch;

  try {
    assert.deepEqual(await fetchAgentInvite(STATUS_URL), { state: "pending" });
    assert.deepEqual(await fetchAgentInvite(STATUS_URL), { state: "pending" });
  } finally {
    globalThis.fetch = previousFetch;
  }
});

test("fetchAgentInvite treats network failure as pending", async () => {
  const previousFetch = globalThis.fetch;
  globalThis.fetch = (async () => {
    throw new Error("connect ECONNREFUSED");
  }) as typeof fetch;

  try {
    assert.deepEqual(await fetchAgentInvite(STATUS_URL), { state: "pending" });
  } finally {
    globalThis.fetch = previousFetch;
  }
});

test("fetchAgentInvite times out slow runtimes and reads pending", async () => {
  const previousFetch = globalThis.fetch;
  globalThis.fetch = ((_input: RequestInfo | URL, init?: RequestInit) =>
    new Promise<Response>((_resolve, reject) => {
      // AbortSignal.timeout uses an unref'ed timer in Node; keep the event
      // loop alive until the abort fires (or fail the test if it never does).
      const keepAlive = setTimeout(() => reject(new Error("abort never fired")), 5_000);
      init?.signal?.addEventListener("abort", () => {
        clearTimeout(keepAlive);
        reject(init.signal?.reason ?? new Error("aborted"));
      });
    })) as typeof fetch;

  try {
    assert.deepEqual(await fetchAgentInvite(STATUS_URL, { timeoutMs: 20 }), {
      state: "pending",
    });
  } finally {
    globalThis.fetch = previousFetch;
  }
});

test("fetchAgentInvite never fetches non-http invite status URLs", async () => {
  const previousFetch = globalThis.fetch;
  globalThis.fetch = (async () => {
    throw new Error("fetch must not be called");
  }) as typeof fetch;

  try {
    for (const url of [null, undefined, "", "   ", "ftp://x", "not a url", INVITE_URL]) {
      assert.deepEqual(await fetchAgentInvite(url), { state: "pending" }, String(url));
    }
  } finally {
    globalThis.fetch = previousFetch;
  }
});

test("resolveAgentInviteDisplay renders ready invites immediately", () => {
  assert.deepEqual(
    resolveAgentInviteDisplay({
      invite: { state: "ready", inviteUrl: INVITE_URL, roomId: "room-1" },
      waitStartedAtMs: null,
      nowMs: READY_AT_MS,
    }),
    { kind: "ready", inviteUrl: INVITE_URL, roomId: "room-1" }
  );
});

test("resolveAgentInviteDisplay renders paired agents without polling, regardless of wait window", () => {
  for (const waitStartedAtMs of [null, READY_AT_MS]) {
    assert.deepEqual(
      resolveAgentInviteDisplay({
        invite: { state: "paired", roomId: "room-1", agentNpub: "npub1agentexamplekey" },
        waitStartedAtMs,
        nowMs: READY_AT_MS + AGENT_INVITE_WAIT_TIMEOUT_MS + 60_000,
      }),
      { kind: "paired", roomId: "room-1", agentNpub: "npub1agentexamplekey" },
      String(waitStartedAtMs)
    );
  }
});

test("resolveAgentInviteDisplay bounds the pending wait", () => {
  // First pending render: stamp the wait window.
  assert.deepEqual(
    resolveAgentInviteDisplay({
      invite: { state: "pending" },
      waitStartedAtMs: null,
      nowMs: READY_AT_MS,
    }),
    { kind: "stamp-wait-start" }
  );

  // Inside the window: keep the waiting panel polling.
  assert.deepEqual(
    resolveAgentInviteDisplay({
      invite: { state: "pending" },
      waitStartedAtMs: READY_AT_MS,
      nowMs: READY_AT_MS + AGENT_INVITE_WAIT_TIMEOUT_MS - 1,
    }),
    { kind: "waiting", deadlineAtMs: READY_AT_MS + AGENT_INVITE_WAIT_TIMEOUT_MS }
  );

  // At and past the deadline: stop polling and offer a manual re-check.
  for (const elapsedMs of [AGENT_INVITE_WAIT_TIMEOUT_MS, AGENT_INVITE_WAIT_TIMEOUT_MS + 60_000]) {
    assert.deepEqual(
      resolveAgentInviteDisplay({
        invite: { state: "pending" },
        waitStartedAtMs: READY_AT_MS,
        nowMs: READY_AT_MS + elapsedMs,
      }),
      { kind: "wait-timeout" },
      `${elapsedMs}ms`
    );
  }
});

test("resolveAgentInviteDisplay surfaces invite errors regardless of the wait window", () => {
  for (const waitStartedAtMs of [null, READY_AT_MS]) {
    assert.deepEqual(
      resolveAgentInviteDisplay({
        invite: { state: "error", message: "invite bridge crashed" },
        waitStartedAtMs,
        nowMs: READY_AT_MS + 5_000,
      }),
      { kind: "error", message: "invite bridge crashed" },
      String(waitStartedAtMs)
    );
  }
});

test("agentInviteWaitStampRedirectPath round-trips through the URL parsers", () => {
  const path = agentInviteWaitStampRedirectPath("machine one", READY_AT_MS);
  assert.equal(
    path,
    `/dashboard/machines/machine%20one?inviteWaitStartedAt=${READY_AT_MS}`
  );

  const params = new URL(path, "https://finite.computer").searchParams;
  assert.equal(parseAgentInviteWaitStartedAt(params.get("inviteWaitStartedAt")), READY_AT_MS);
});

test("parseAgentInviteWaitStartedAt accepts only positive epoch integers", () => {
  assert.equal(parseAgentInviteWaitStartedAt(String(READY_AT_MS)), READY_AT_MS);
  assert.equal(parseAgentInviteWaitStartedAt("0"), null);
  assert.equal(parseAgentInviteWaitStartedAt("-2"), null);
  assert.equal(parseAgentInviteWaitStartedAt("1.5"), null);
  assert.equal(parseAgentInviteWaitStartedAt("soon"), null);
  assert.equal(parseAgentInviteWaitStartedAt(""), null);
  assert.equal(parseAgentInviteWaitStartedAt(null), null);
  assert.equal(parseAgentInviteWaitStartedAt(undefined), null);
});

test("truncateInviteUrl keeps short URIs and truncates long ones", () => {
  assert.equal(truncateInviteUrl("finite://join?x=1"), "finite://join?x=1");
  const truncated = truncateInviteUrl(INVITE_URL, 20);
  assert.equal(truncated, `${INVITE_URL.slice(0, 20)}…`);
});

test("truncateNpub keeps short values and middle-truncates long ones", () => {
  assert.equal(truncateNpub("npub1short"), "npub1short");
  const npub = "npub1qqqsyv9fyxnrqe6anwcvxlgxxtnwhyq0gkhwlgkflspyw2wxr3qzk6qy7d2v4";
  assert.equal(truncateNpub(npub), `${npub.slice(0, 12)}…${npub.slice(-6)}`);
});
