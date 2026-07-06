import assert from "node:assert/strict";
import { test } from "node:test";

import { accountFromWorkosSessionCookie, getAccountAuthContext } from "./dashboard-auth";

function unsignedJwt(payload: object) {
  return [
    Buffer.from(JSON.stringify({ alg: "none", typ: "JWT" })).toString("base64url"),
    Buffer.from(JSON.stringify(payload)).toString("base64url"),
    "",
  ].join(".");
}

test("WorkOS session cookie account fallback extracts a usable verified identity", () => {
  assert.deepEqual(
    accountFromWorkosSessionCookie(
      {
        accessToken: unsignedJwt({ exp: 2_000 }),
        user: {
          id: "user_123",
          email: "Paul@Finite.Vip",
          emailVerified: true,
        },
      },
      1_000_000
    ),
    {
      email: "paul@finite.vip",
      workosUserId: "user_123",
      emailVerified: true,
      source: "workos",
    }
  );
});

test("WorkOS session cookie account fallback rejects expired sessions", () => {
  assert.equal(
    accountFromWorkosSessionCookie(
      {
        accessToken: unsignedJwt({ exp: 1_000 }),
        user: {
          id: "user_123",
          email: "paul@finite.vip",
          emailVerified: true,
        },
      },
      1_000_001
    ),
    null
  );
});

test("WorkOS session cookie account fallback keeps unverified email state explicit", () => {
  assert.deepEqual(
    accountFromWorkosSessionCookie(
      {
        accessToken: unsignedJwt({ exp: 2_000 }),
        user: {
          id: "user_123",
          email: "paul@finite.vip",
          emailVerified: false,
        },
      },
      1_000_000
    ),
    {
      email: "paul@finite.vip",
      workosUserId: "user_123",
      emailVerified: false,
      source: "workos",
    }
  );
});

test("dev identity override provides a verified dev account for browser tests", async () => {
  const previousEmail = process.env.FC_DASHBOARD_DEV_EMAIL;
  const previousUserId = process.env.FC_DASHBOARD_DEV_WORKOS_USER_ID;
  const previousWorkosEnabled = process.env.FC_WORKOS_AUTH_ENABLED;

  process.env.FC_DASHBOARD_DEV_EMAIL = "Browser@Finite.VIP";
  process.env.FC_DASHBOARD_DEV_WORKOS_USER_ID = "user_browser";
  delete process.env.FC_WORKOS_AUTH_ENABLED;

  try {
    assert.deepEqual(await getAccountAuthContext(), {
      email: "browser@finite.vip",
      workosUserId: "user_browser",
      emailVerified: true,
      source: "dev",
    });
  } finally {
    if (previousEmail === undefined) {
      delete process.env.FC_DASHBOARD_DEV_EMAIL;
    } else {
      process.env.FC_DASHBOARD_DEV_EMAIL = previousEmail;
    }
    if (previousUserId === undefined) {
      delete process.env.FC_DASHBOARD_DEV_WORKOS_USER_ID;
    } else {
      process.env.FC_DASHBOARD_DEV_WORKOS_USER_ID = previousUserId;
    }
    if (previousWorkosEnabled === undefined) {
      delete process.env.FC_WORKOS_AUTH_ENABLED;
    } else {
      process.env.FC_WORKOS_AUTH_ENABLED = previousWorkosEnabled;
    }
  }
});
