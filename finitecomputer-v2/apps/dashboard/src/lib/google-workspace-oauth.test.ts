import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import {
  acceptedGoogleWorkspaceScopes,
  googleWorkspaceOAuthConfig,
  googleWorkspaceOAuthConfigured,
  googleWorkspaceDashboardUrl,
  GOOGLE_WORKSPACE_SCOPES,
  sealGoogleWorkspaceState,
  unsealGoogleWorkspaceState,
} from "@/lib/google-workspace-oauth";

const env = {
  GOOGLE_WORKSPACE_CLIENT_ID: "client-id",
  GOOGLE_WORKSPACE_CLIENT_SECRET: "client-secret",
  WORKOS_COOKIE_PASSWORD: "12345678901234567890123456789012",
};

test("Google Workspace OAuth uses the dashboard callback and sealed user-bound state", async () => {
  assert.equal(googleWorkspaceOAuthConfigured(env), true);
  assert.deepEqual(googleWorkspaceOAuthConfig("http://127.0.0.1:13002/path", env), {
    clientId: "client-id",
    clientSecret: "client-secret",
    redirectUri: "http://127.0.0.1:13002/google-workspace/callback",
  });
  const state = {
    machineId: "machine-a",
    workosUserId: "user-a",
    issuedAtMs: Date.now(),
  };
  const sealed = await sealGoogleWorkspaceState(state, env);
  assert.deepEqual(await unsealGoogleWorkspaceState(sealed, env), state);
  assert.equal(
    await unsealGoogleWorkspaceState(sealed, {
      ...env,
      WORKOS_COOKIE_PASSWORD: "abcdefghijklmnopqrstuvwxyz123456",
    }),
    null
  );
});

test("Google Workspace OAuth derives the exact production callback from the configured public origin", () => {
  const productionEnv = {
    ...env,
    FC_DASHBOARD_BASE_URL: "https://finite.computer",
    NEXT_PUBLIC_WORKOS_REDIRECT_URI: "https://finite.computer/callback",
  };
  assert.equal(
    googleWorkspaceOAuthConfig("http://127.0.0.1:3000/internal", productionEnv)?.redirectUri,
    "https://finite.computer/google-workspace/callback"
  );
  assert.equal(
    googleWorkspaceDashboardUrl(
      "/dashboard/machines/runtime-a/connections?google=connected",
      "http://127.0.0.1:3000/google-workspace/callback",
      productionEnv
    ).toString(),
    "https://finite.computer/dashboard/machines/runtime-a/connections?google=connected"
  );
});

test("dashboard and installed skill share one Google Workspace scope contract", async () => {
  const skillScopes = JSON.parse(
    await readFile(
      path.resolve(
        process.cwd(),
        "../../../finite-skills/skills/productivity/google-workspace-finite/references/google-workspace-scopes.json"
      ),
      "utf8"
    )
  );
  assert.deepEqual(GOOGLE_WORKSPACE_SCOPES, skillScopes);
});

const DRIVE = "https://www.googleapis.com/auth/drive";
const GMAIL_SEND = "https://www.googleapis.com/auth/gmail.send";
const USERINFO_EMAIL = "https://www.googleapis.com/auth/userinfo.email";
const USERINFO_PROFILE = "https://www.googleapis.com/auth/userinfo.profile";

test("Google Workspace accepts a partial grant that still binds the account email", () => {
  const withoutSend = GOOGLE_WORKSPACE_SCOPES.filter((scope) => scope !== GMAIL_SEND);
  assert.deepEqual(acceptedGoogleWorkspaceScopes(GOOGLE_WORKSPACE_SCOPES.join(" ")), [
    ...GOOGLE_WORKSPACE_SCOPES,
  ]);
  assert.deepEqual(acceptedGoogleWorkspaceScopes([...withoutSend].reverse().join("  ")), withoutSend);
  assert.deepEqual(acceptedGoogleWorkspaceScopes(`openid ${USERINFO_EMAIL} ${DRIVE} ${DRIVE}`), [
    DRIVE,
    "openid",
    USERINFO_EMAIL,
  ]);
});

test("Google Workspace rejects grants outside the contract or without Workspace access", () => {
  const identity = `openid ${USERINFO_EMAIL}`;
  assert.equal(
    acceptedGoogleWorkspaceScopes(`${identity} ${DRIVE} https://www.googleapis.com/auth/script.projects`),
    null
  );
  assert.equal(acceptedGoogleWorkspaceScopes(`openid ${DRIVE}`), null);
  assert.equal(acceptedGoogleWorkspaceScopes(`${USERINFO_EMAIL} ${DRIVE}`), null);
  assert.equal(acceptedGoogleWorkspaceScopes(`${identity} ${USERINFO_PROFILE}`), null);
  assert.equal(acceptedGoogleWorkspaceScopes(""), null);
});
