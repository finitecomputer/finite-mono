"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");

const {
  completeDeviceSecretGrant,
  targetEnrollmentIsReady,
  waitForSourceEnrollment,
  waitForTargetEnrollment,
} = require("./device-link-orchestration.cjs");

function response(status) {
  return {
    pairing_session_id: "pairing-deadlock-regression",
    target_device_id: "electron-deadlock-regression",
    status,
    expires_at_unix_seconds: 2_000,
    room_count: 0,
    active_room_count: 0,
    source_descriptor: {
      version: 1,
      source_public_key: "source-key",
      relay_url: "https://chat.example.test",
      expires_at_unix_seconds: 2_000,
    },
  };
}

function parseResponse(value, expected) {
  assert.equal(value.pairing_session_id, expected.pairing_session_id);
  assert.equal(value.target_device_id, expected.target_device_id);
  return value;
}

function enrollmentGrant() {
  return {
    pairing_session_id: "pairing-deadlock-regression",
    target_device_id: "electron-deadlock-regression",
    enrollment_user_id: "user_test",
    enrollment_capability_hex: "ab".repeat(32),
  };
}

test("status polling drives the source before the Rust target can complete", async () => {
  const request = {
    pairing_session_id: "pairing-deadlock-regression",
    target_device_id: "electron-deadlock-regression",
  };
  let acceptDescriptor = null;
  let storeTarget;
  const link = {
    durable: new Promise((resolve) => {
      storeTarget = resolve;
    }),
    completion: new Promise(() => {}),
    acceptSourceDescriptor(descriptor) {
      acceptDescriptor = descriptor;
    },
  };
  let polls = 0;

  await completeDeviceSecretGrant({
    link,
    request,
    approved: response("awaiting_offer"),
    parseResponse,
    pollStatus: async () => {
      polls += 1;
      assert.ok(acceptDescriptor, "the target receives the descriptor before source polling");
      storeTarget(enrollmentGrant());
      return response("grant_available");
    },
    now: () => 1_000_000,
    delay: async () => {},
    pollIntervalMs: 0,
  });

  assert.equal(polls, 1, "the source is pumped while target completion is pending");
});

test("the bounded grant retries transient source polling failures", async () => {
  const request = {
    pairing_session_id: "pairing-deadlock-regression",
    target_device_id: "electron-deadlock-regression",
  };
  let storeTarget;
  const link = {
    durable: new Promise((resolve) => {
      storeTarget = resolve;
    }),
    completion: new Promise(() => {}),
    acceptSourceDescriptor() {},
  };
  let polls = 0;

  await completeDeviceSecretGrant({
    link,
    request,
    approved: response("awaiting_offer"),
    parseResponse,
    pollStatus: async () => {
      polls += 1;
      if (polls === 1) throw new Error("transient dashboard failure");
      storeTarget(enrollmentGrant());
      return response("grant_available");
    },
    now: () => 1_000_000,
    delay: async () => {},
    pollIntervalMs: 0,
  });

  assert.equal(polls, 2);
});

test("durable identity is success even when best-effort NIP Complete later fails", async () => {
  const request = {
    pairing_session_id: "pairing-deadlock-regression",
    target_device_id: "electron-deadlock-regression",
  };
  let storeTarget;
  let rejectCompletion;
  const link = {
    durable: new Promise((resolve) => {
      storeTarget = resolve;
    }),
    completion: new Promise((_resolve, reject) => {
      rejectCompletion = reject;
    }),
    acceptSourceDescriptor() {},
  };

  const stored = await completeDeviceSecretGrant({
    link,
    request,
    approved: response("awaiting_offer"),
    parseResponse,
    pollStatus: async () => {
      storeTarget(enrollmentGrant());
      return response("grant_available");
    },
    now: () => 1_000_000,
    delay: async () => {},
    pollIntervalMs: 0,
  });
  assert.deepEqual(stored, enrollmentGrant());

  rejectCompletion(new Error("relay rejected best-effort complete"));
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(stored, enrollmentGrant());
});

test("durable identity does not wait for an in-flight status request before succeeding", async () => {
  const request = {
    pairing_session_id: "pairing-deadlock-regression",
    target_device_id: "electron-deadlock-regression",
  };
  let storeTarget;
  const link = {
    durable: new Promise((resolve) => {
      storeTarget = resolve;
    }),
    completion: new Promise(() => {}),
    acceptSourceDescriptor() {
      queueMicrotask(() => storeTarget(enrollmentGrant()));
    },
  };
  const stored = await completeDeviceSecretGrant({
    link,
    request,
    approved: response("awaiting_offer"),
    parseResponse,
    pollStatus: async () => new Promise(() => {}),
    now: () => 1_000_000,
    delay: async () => {},
    pollIntervalMs: 0,
  });
  assert.deepEqual(stored, enrollmentGrant());
});

test("the bounded grant fails immediately on permanent dashboard rejection", async () => {
  const request = {
    pairing_session_id: "pairing-deadlock-regression",
    target_device_id: "electron-deadlock-regression",
  };
  const terminal = Object.assign(new Error("sign in is no longer valid"), {
    retryable: false,
  });
  let polls = 0;
  const link = {
    durable: new Promise(() => {}),
    completion: new Promise(() => {}),
    acceptSourceDescriptor() {},
  };
  await assert.rejects(
    completeDeviceSecretGrant({
      link,
      request,
      approved: response("awaiting_offer"),
      parseResponse,
      pollStatus: async () => {
        polls += 1;
        throw terminal;
      },
      isRetryableError: (error) => error?.retryable === true,
      now: () => 1_000_000,
      delay: async () => {},
      pollIntervalMs: 0,
    }),
    /sign in is no longer valid/u
  );
  assert.equal(polls, 1);
});

function enrolledState(overrides = {}) {
  const manifest = {
    bootstrap_id: "pairing-deadlock-regression",
    room_id: "room-agent",
    manifest_sha256: "ab".repeat(32),
  };
  return {
    identity: {
      account_id: "11".repeat(32),
      device_id: "electron-deadlock-regression",
    },
    device_link_bootstrap_receipts: [manifest],
    paired_agent: {
      agent_account_id: "agent",
      canonical_room_id: "room-agent",
    },
    rooms: [
      {
        room_id: "room-agent",
        display_name: "Hermes",
        state: "connected",
        is_agent_chat: true,
      },
      {
        room_id: "room-notes",
        display_name: "Notes",
        state: "connected",
        is_agent_chat: false,
      },
    ],
    ...overrides,
  };
}

test("target readiness requires exact room hydration and canonical agent pairing", () => {
  const manifests = enrolledState().device_link_bootstrap_receipts;
  assert.equal(
    targetEnrollmentIsReady(
      enrolledState(),
      "11".repeat(32),
      "electron-deadlock-regression",
      manifests
    ),
    true
  );
  assert.equal(
    targetEnrollmentIsReady(
      enrolledState({ device_link_bootstrap_receipts: [] }),
      "11".repeat(32),
      "electron-deadlock-regression",
      manifests
    ),
    false,
    "room counts and display names cannot substitute for the exact manifest"
  );
  assert.equal(
    targetEnrollmentIsReady(
      enrolledState({ paired_agent: null }),
      "11".repeat(32),
      "electron-deadlock-regression",
      manifests
    ),
    false,
    "source fanout alone is not target readiness"
  );
  assert.equal(
    targetEnrollmentIsReady(
      enrolledState(),
      "22".repeat(32),
      "electron-deadlock-regression",
      manifests
    ),
    false,
    "the durable receipt cannot enroll the wrong account"
  );
});

test("target enrollment waits through partial sync and returns only converged state", async () => {
  const states = [
    enrolledState({ rooms: [enrolledState().rooms[0]], paired_agent: null }),
    enrolledState({
      device_link_bootstrap_receipts: [],
      rooms: [
        enrolledState().rooms[0],
        { ...enrolledState().rooms[1], display_name: "room-notes" },
      ],
    }),
    enrolledState(),
  ];
  let reads = 0;
  const result = await waitForTargetEnrollment({
    expectedAccountId: "11".repeat(32),
    expectedDeviceId: "electron-deadlock-regression",
    expectedManifests: enrolledState().device_link_bootstrap_receipts,
    readState: async () => states[Math.min(reads++, states.length - 1)],
    delay: async () => {},
    pollIntervalMs: 0,
  });
  assert.equal(reads, 3);
  assert.deepEqual(result, states[2]);
});

test("target enrollment has a typed terminal bound instead of an infinite spinner", async () => {
  let time = 0;
  await assert.rejects(
    waitForTargetEnrollment({
      expectedAccountId: "11".repeat(32),
      expectedDeviceId: "electron-deadlock-regression",
      expectedManifests: enrolledState().device_link_bootstrap_receipts,
      readState: async () => enrolledState({ paired_agent: null }),
      now: () => time++,
      delay: async () => {},
      pollIntervalMs: 0,
      timeoutMs: 2,
    }),
    /complete chat history/u
  );
});

test("durable source enrollment retries transport failures and returns only ready", async () => {
  const request = enrollmentGrant();
  const results = [
    new Error("transient dashboard failure"),
    response("awaiting_key_package"),
    response("joining_rooms"),
    response("ready"),
  ];
  let polls = 0;
  let targetTicks = 0;
  const progress = [];
  const ready = await waitForSourceEnrollment({
    request,
    advanceTarget: async () => {
      targetTicks += 1;
    },
    pollEnrollment: async () => {
      const result = results[polls++];
      if (result instanceof Error) throw result;
      return result;
    },
    parseResponse,
    reportStatus: (status) => progress.push(status),
    delay: async () => {},
    pollIntervalMs: 0,
  });
  assert.equal(polls, 4);
  assert.equal(
    targetTicks,
    4,
    "every source poll is preceded by a target sync/KeyPackage replenishment tick"
  );
  assert.equal(ready.status, "ready");
  assert.deepEqual(progress, [
    {
      status: "joining_rooms",
      message: "A temporary interruption occurred. Retrying automatically…",
    },
    { status: "joining_rooms" },
    { status: "joining_rooms" },
  ]);
});

test("durable source enrollment has a typed terminal bound", async () => {
  let time = 0;
  await assert.rejects(
    waitForSourceEnrollment({
      request: enrollmentGrant(),
      pollEnrollment: async () => response("joining_rooms"),
      parseResponse,
      now: () => time++,
      delay: async () => {},
      pollIntervalMs: 0,
      timeoutMs: 2,
    }),
    /complete chat history/u
  );
});

test("durable source enrollment fails immediately on a terminal capability rejection", async () => {
  const terminal = Object.assign(new Error("capability rejected"), {
    retryable: false,
  });
  let polls = 0;
  await assert.rejects(
    waitForSourceEnrollment({
      request: enrollmentGrant(),
      pollEnrollment: async () => {
        polls += 1;
        throw terminal;
      },
      parseResponse,
      isRetryableError: (error) => error?.retryable === true,
      delay: async () => {},
      pollIntervalMs: 0,
    }),
    /capability rejected/u
  );
  assert.equal(polls, 1);
});
