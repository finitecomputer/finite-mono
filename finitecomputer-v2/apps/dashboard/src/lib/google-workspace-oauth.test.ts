import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import {
  googleWorkspaceOAuthConfig,
  googleWorkspaceOAuthConfigured,
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
