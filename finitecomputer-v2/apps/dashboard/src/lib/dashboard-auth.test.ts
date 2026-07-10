import assert from "node:assert/strict";
import { test } from "node:test";

import {
  accountFromWorkosSessionCookie,
  devAccountAuthContext,
  getAccountAuthContext,
} from "./dashboard-auth";

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
      accessToken: unsignedJwt({ exp: 2_000 }),
      organizationId: null,
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
      accessToken: unsignedJwt({ exp: 2_000 }),
      organizationId: null,
      source: "workos",
    }
  );
});

test("dev identity override provides a verified dev account for browser tests", async () => {
  const previousEmail = process.env.FC_DASHBOARD_DEV_EMAIL;
  const previousUserId = process.env.FC_DASHBOARD_DEV_WORKOS_USER_ID;
  const previousWorkosEnabled = process.env.FC_WORKOS_AUTH_ENABLED;
  const previousAllowDevAuth = process.env.FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH;
  const previousAccessToken = process.env.FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN;

  process.env.FC_DASHBOARD_DEV_EMAIL = "Browser@Finite.VIP";
  process.env.FC_DASHBOARD_DEV_WORKOS_USER_ID = "user_browser";
  process.env.FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN = unsignedJwt({ exp: 2_000 });
  process.env.FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH = "1";
  delete process.env.FC_WORKOS_AUTH_ENABLED;

  try {
    assert.deepEqual(await getAccountAuthContext(), {
      email: "browser@finite.vip",
      workosUserId: "user_browser",
      emailVerified: true,
      accessToken: unsignedJwt({ exp: 2_000 }),
      organizationId: null,
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
    if (previousAllowDevAuth === undefined) {
      delete process.env.FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH;
    } else {
      process.env.FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH = previousAllowDevAuth;
    }
    if (previousAccessToken === undefined) {
      delete process.env.FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN;
    } else {
      process.env.FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN = previousAccessToken;
    }
  }
});

test("dev identity fails closed without both the explicit flag and fixture token", () => {
  const configured = {
    FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
    FC_DASHBOARD_DEV_EMAIL: "dev@finite.vip",
    FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_dev",
    FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: unsignedJwt({ exp: 2_000 }),
  };

  assert.equal(
    devAccountAuthContext({
      ...configured,
      FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: undefined,
    }),
    null
  );
  assert.equal(
    devAccountAuthContext({
      ...configured,
      FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: undefined,
    }),
    null
  );
});
