import assert from "node:assert/strict";
import { test } from "node:test";

import {
  missingWorkosAuthEnv,
  safeWorkosReturnPathname,
  workosAuthEnabled,
  workosAuthStatus,
  workosBaseUrl,
  workosInteractiveAuthRequest,
  workosLogoutReturnTo,
  workosProtectedPath,
  workosProxyBypassPath,
  workosSessionCookieName,
} from "./workos-auth";

test("WorkOS auth is disabled unless the explicit rollout flag is truthy", () => {
  assert.equal(workosAuthEnabled({}), false);
  assert.equal(workosAuthEnabled({ FC_WORKOS_AUTH_ENABLED: "false" }), false);
  assert.equal(workosAuthEnabled({ FC_WORKOS_AUTH_ENABLED: "true" }), true);
  assert.equal(workosAuthEnabled({ FC_WORKOS_AUTH_ENABLED: "1" }), true);
});

test("WorkOS readiness reports required environment without enabling auth implicitly", () => {
  assert.deepEqual(workosAuthStatus({}), {
    enabled: false,
    ready: false,
    missing: [],
  });

  assert.deepEqual(
    workosAuthStatus({
      FC_WORKOS_AUTH_ENABLED: "1",
      WORKOS_API_KEY: "sk_test",
      WORKOS_CLIENT_ID: "client_test",
      WORKOS_COOKIE_PASSWORD: "short",
      NEXT_PUBLIC_WORKOS_REDIRECT_URI: "http://localhost:3000/callback",
    }),
    {
      enabled: true,
      ready: false,
      missing: ["WORKOS_COOKIE_PASSWORD>=32"],
    },
  );

  assert.deepEqual(
    workosAuthStatus({
      FC_WORKOS_AUTH_ENABLED: "1",
      WORKOS_API_KEY: "sk_test",
      WORKOS_CLIENT_ID: "client_test",
      WORKOS_COOKIE_PASSWORD: "12345678901234567890123456789012",
      NEXT_PUBLIC_WORKOS_REDIRECT_URI: "not a url",
    }),
    {
      enabled: true,
      ready: false,
      missing: ["NEXT_PUBLIC_WORKOS_REDIRECT_URI(valid URL)"],
    },
  );

  assert.deepEqual(
    missingWorkosAuthEnv({
      WORKOS_API_KEY: "sk_test",
      WORKOS_CLIENT_ID: "client_test",
      WORKOS_COOKIE_PASSWORD: "12345678901234567890123456789012",
      NEXT_PUBLIC_WORKOS_REDIRECT_URI: "https://finite.computer/callback",
    }),
    [],
  );
});

test("WorkOS proxy protects app surfaces but leaves entry and callback routes public", () => {
  assert.equal(workosProtectedPath("/"), false);
  assert.equal(workosProtectedPath("/login"), false);
  assert.equal(workosProtectedPath("/signup"), false);
  assert.equal(workosProtectedPath("/callback"), false);
  assert.equal(workosProtectedPath("/health"), false);
  assert.equal(workosProtectedPath("/api/finite/v1/heartbeat"), false);
  assert.equal(workosProtectedPath("/api/stripe/webhook"), false);
  assert.equal(workosProtectedPath("/dashboard/agent-creation-requests"), true);
  assert.equal(workosProtectedPath("/dashboard"), true);
  assert.equal(workosProtectedPath("/dashboard/machines/smoke"), true);
  assert.equal(workosProtectedPath("/client"), true);
  assert.equal(workosProtectedPath("/client/app.js"), true);
  assert.equal(workosProtectedPath("/_admin/vaults"), false);
  assert.equal(workosProtectedPath("/api/pwa/manifest"), true);
  assert.equal(workosProtectedPath("/claim/token"), false);
});

test("WorkOS proxy bypasses auth endpoints and unauthenticated runtime callbacks", () => {
  assert.equal(workosProxyBypassPath("/"), false);
  assert.equal(workosProxyBypassPath("/dashboard"), false);
  assert.equal(workosProxyBypassPath("/dashboard/agent-creation-requests"), false);
  assert.equal(workosProxyBypassPath("/login"), true);
  assert.equal(workosProxyBypassPath("/signup"), true);
  assert.equal(workosProxyBypassPath("/callback"), true);
  assert.equal(workosProxyBypassPath("/logout"), true);
  assert.equal(workosProxyBypassPath("/api/brain/identity-provider"), true);
  assert.equal(workosProxyBypassPath("/api/brain/session-proof"), false);
  assert.equal(workosProxyBypassPath("/health"), true);
  assert.equal(workosProxyBypassPath("/_admin"), true);
  assert.equal(workosProxyBypassPath("/_admin/vaults"), true);
  assert.equal(workosProxyBypassPath("/client"), false);
  assert.equal(workosProxyBypassPath("/api/finite/v1/heartbeat"), true);
  assert.equal(workosProxyBypassPath("/api/stripe/webhook"), true);
});

test("WorkOS auth redirects only for interactive browser navigations", () => {
  assert.equal(
    workosInteractiveAuthRequest(
      "GET",
      new Headers({
        accept: "text/html,application/xhtml+xml",
        "sec-fetch-mode": "navigate",
      })
    ),
    true
  );
  assert.equal(
    workosInteractiveAuthRequest(
      "GET",
      new Headers({
        accept: "text/html,application/xhtml+xml",
        "next-router-prefetch": "1",
      })
    ),
    false
  );
  assert.equal(
    workosInteractiveAuthRequest(
      "POST",
      new Headers({
        accept: "text/html,application/xhtml+xml",
        "sec-fetch-mode": "navigate",
      })
    ),
    true
  );
  assert.equal(
    workosInteractiveAuthRequest(
      "GET",
      new Headers({
        accept: "text/x-component",
        rsc: "1",
      })
    ),
    false
  );
  assert.equal(
    workosInteractiveAuthRequest(
      "POST",
      new Headers({
        accept: "*/*",
        "sec-fetch-mode": "cors",
      })
    ),
    false
  );
});

test("WorkOS session cookie name defaults to the SDK cookie", () => {
  assert.equal(workosSessionCookieName({}), "wos-session");
  assert.equal(workosSessionCookieName({ WORKOS_COOKIE_NAME: "finite-session" }), "finite-session");
});

test("WorkOS return paths stay app-relative", () => {
  assert.equal(safeWorkosReturnPathname(null), "/dashboard");
  assert.equal(safeWorkosReturnPathname("/dashboard?machine=smoke"), "/dashboard?machine=smoke");
  assert.equal(safeWorkosReturnPathname("https://evil.example/dashboard"), "/dashboard");
  assert.equal(safeWorkosReturnPathname("//evil.example/dashboard"), "/dashboard");
});

test("WorkOS callback base URL is derived from the public redirect URI", () => {
  assert.equal(
    workosBaseUrl({
      NEXT_PUBLIC_WORKOS_REDIRECT_URI: "https://finite.computer/callback",
    }),
    "https://finite.computer",
  );
  assert.equal(workosBaseUrl({ NEXT_PUBLIC_WORKOS_REDIRECT_URI: "not a url" }), undefined);
  assert.equal(workosBaseUrl({}), undefined);
});

test("WorkOS logout return URL is absolute for deployed environments", () => {
  assert.equal(
    workosLogoutReturnTo({
      NEXT_PUBLIC_WORKOS_REDIRECT_URI: "https://finite.computer/callback",
    }),
    "https://finite.computer/",
  );
  assert.equal(workosLogoutReturnTo({}), "/");
});
