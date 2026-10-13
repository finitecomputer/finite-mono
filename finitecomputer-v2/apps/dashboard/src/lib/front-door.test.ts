import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import type {
  CoreAgentCreationRequestSummary,
  CoreMe,
  CoreVisibleProject,
} from "./core-client";
import {
  FRONT_DOOR_DEFAULT_SIGNUP_URL,
  FRONT_DOOR_DEFAULT_TARGET_URL,
  accountMigrated,
  frontDoorAgentCreationMovedMessage,
  frontDoorBannerHref,
  frontDoorConfig,
  frontDoorDecision,
  frontDoorOffersAgentCreation,
  hasLegacyAgent,
  type FrontDoorAccount,
  type FrontDoorConfig,
} from "./front-door";
import { createFrontDoorBannerStore, FRONT_DOOR_BANNER_DISMISSED_KEY } from "./front-door-banner";

const ENABLED: FrontDoorConfig = {
  enabled: true,
  targetUrl: FRONT_DOOR_DEFAULT_TARGET_URL,
  signupUrl: FRONT_DOOR_DEFAULT_SIGNUP_URL,
};
const DISABLED: FrontDoorConfig = { enabled: false, invalid: [] };
const PRODUCTION_ENV = {
  FC_FRONT_DOOR_ENABLED: "true",
  NEXT_PUBLIC_WORKOS_REDIRECT_URI: "https://finite.computer/callback",
  FC_DASHBOARD_BASE_URL: "https://finite.computer",
};

function project(id: string, runtime: boolean, extra: Record<string, unknown> = {}): CoreVisibleProject {
  return {
    project: {
      id,
      display_name: id,
      created_at: "2026-10-01T00:00:00Z",
      updated_at: "2026-10-01T00:00:00Z",
      ...extra,
    },
    runtime: runtime
      ? {
          id: `runtime_${id}`,
          project_id: id,
          runtime_status: "unknown",
          created_at: "2026-10-01T00:00:00Z",
          updated_at: "2026-10-01T00:00:00Z",
        }
      : null,
  };
}

function request(
  status: string,
  agentRuntimeId: string | null = null,
  extra: Partial<CoreAgentCreationRequestSummary> = {}
): CoreAgentCreationRequestSummary {
  return {
    id: `request_${status}`,
    project_id: "project_request",
    display_name: "Request",
    status: status as CoreAgentCreationRequestSummary["status"],
    agent_runtime_id: agentRuntimeId,
    created_at: "2026-10-01T00:00:00Z",
    updated_at: "2026-10-01T00:00:00Z",
    ...extra,
  };
}

function me(overrides: Partial<CoreMe> = {}): CoreMe {
  return {
    email: "owner@example.test",
    workos_user_id: "user_owner",
    projects: [],
    agent_creation_requests: [],
    ...overrides,
  };
}

function account(overrides: Partial<FrontDoorAccount> = {}): FrontDoorAccount {
  return {
    isAdmin: false,
    workosUserId: "user_owner",
    me: me(),
    coreUnavailable: false,
    paidSetupPending: false,
    ...overrides,
  };
}

test("the front door is off unless FC_FRONT_DOOR_ENABLED is exactly true", () => {
  for (const value of [undefined, "", "false", "0", "1", "yes", "on", "enabled"]) {
    assert.deepEqual(
      frontDoorConfig({ ...PRODUCTION_ENV, FC_FRONT_DOOR_ENABLED: value }),
      DISABLED,
      `${value} must not enable the front door`
    );
  }
  for (const value of ["true", " TRUE ", "True"]) {
    assert.equal(frontDoorConfig({ ...PRODUCTION_ENV, FC_FRONT_DOOR_ENABLED: value }).enabled, true);
  }
  assert.equal(frontDoorOffersAgentCreation(frontDoorConfig({})), true);
  assert.equal(frontDoorOffersAgentCreation(ENABLED), false);
});

test("front door destinations default to finite.chat and accept overrides", () => {
  assert.deepEqual(frontDoorConfig(PRODUCTION_ENV), {
    enabled: true,
    targetUrl: "https://finite.chat/",
    signupUrl: "https://finite.chat/auth/login",
  });
  assert.deepEqual(
    frontDoorConfig({
      ...PRODUCTION_ENV,
      FC_FRONT_DOOR_TARGET_URL: " https://staging.finite.chat/welcome ",
      FC_FRONT_DOOR_SIGNUP_URL: "https://staging.finite.chat/auth/login?from=finite.computer",
    }),
    {
      enabled: true,
      targetUrl: "https://staging.finite.chat/welcome",
      signupUrl: "https://staging.finite.chat/auth/login?from=finite.computer",
    }
  );
  // Loopback http is accepted for local development only.
  assert.equal(
    frontDoorConfig({
      FC_FRONT_DOOR_ENABLED: "true",
      NEXT_PUBLIC_WORKOS_REDIRECT_URI: "http://localhost:3000/callback",
      FC_FRONT_DOOR_TARGET_URL: "http://localhost:3001/",
      FC_FRONT_DOOR_SIGNUP_URL: "http://127.0.0.1:3001/auth/login",
    }).enabled,
    true
  );
});

test("an invalid or self-referential destination keeps the front door off", () => {
  for (const [name, value] of [
    ["FC_FRONT_DOOR_TARGET_URL", "not a url"],
    ["FC_FRONT_DOOR_TARGET_URL", "/relative"],
    ["FC_FRONT_DOOR_TARGET_URL", "http://finite.chat/"],
    ["FC_FRONT_DOOR_TARGET_URL", "javascript:alert(1)"],
    ["FC_FRONT_DOOR_TARGET_URL", "https://user:secret@finite.chat/"],
    // The dashboard's own origin would send accounts back into this router.
    ["FC_FRONT_DOOR_TARGET_URL", "https://finite.computer/"],
    ["FC_FRONT_DOOR_TARGET_URL", "https://finite.computer/dashboard"],
    ["FC_FRONT_DOOR_SIGNUP_URL", "https://finite.computer/signup"],
    ["FC_FRONT_DOOR_SIGNUP_URL", "ftp://finite.chat/auth/login"],
  ] as const) {
    assert.deepEqual(
      frontDoorConfig({ ...PRODUCTION_ENV, [name]: value }),
      { enabled: false, invalid: [name] },
      `${name}=${value}`
    );
  }
  assert.deepEqual(
    frontDoorConfig({
      FC_FRONT_DOOR_ENABLED: "true",
      NEXT_PUBLIC_APP_URL: "https://finite.chat",
    }),
    { enabled: false, invalid: ["FC_FRONT_DOOR_TARGET_URL", "FC_FRONT_DOOR_SIGNUP_URL"] }
  );
});

test("switch off keeps every account on the legacy dashboard exactly as today", () => {
  for (const candidate of [
    account(),
    account({ me: me({ migrated_at: "2026-10-19T09:00:00Z" }) }),
    account({ me: null, coreUnavailable: true }),
  ]) {
    assert.deepEqual(frontDoorDecision(DISABLED, candidate), { kind: "stay", reason: "disabled" });
    assert.equal(frontDoorBannerHref(DISABLED, frontDoorDecision(DISABLED, candidate)), null);
  }
});

test("accounts without a legacy agent go to the new Finite", () => {
  const decision = frontDoorDecision(ENABLED, account());
  assert.deepEqual(decision, {
    kind: "redirect",
    reason: "no-legacy-agent",
    url: "https://finite.chat/",
  });
  assert.equal(frontDoorBannerHref(ENABLED, decision), null);
  // Finished or abandoned creation is not an agent.
  for (const requests of [
    [request("failed")],
    [request("cancelled")],
    // A completed launch whose runtime was later retired (its Project is no
    // longer listed) keeps its assigned runtime id on the request.
    [request("running", "runtime_retired")],
  ]) {
    assert.equal(
      frontDoorDecision(ENABLED, account({ me: me({ projects: [project("p", false)], agent_creation_requests: requests }) })).kind,
      "redirect"
    );
  }
});

test("accounts with a legacy agent stay and see the banner", () => {
  for (const [label, candidate] of [
    ["active runtime link", me({ projects: [project("p", true)] })],
    ["queued creation", me({ projects: [project("p", false)], agent_creation_requests: [request("requested")] })],
    ["launching creation", me({ agent_creation_requests: [request("launching")] })],
    ["running creation awaiting its runtime", me({ agent_creation_requests: [request("running")] })],
    ["in-flight relocation", me({ agent_creation_requests: [request("launching", "runtime_p", { is_relocation: true })] })],
    ["unknown creation status", me({ agent_creation_requests: [request("paused_for_review")] })],
    ["one live agent among failures", me({ projects: [project("a", false), project("b", true)], agent_creation_requests: [request("failed")] })],
  ] as const) {
    const decision = frontDoorDecision(ENABLED, account({ me: candidate }));
    assert.deepEqual(decision, { kind: "stay", reason: "legacy-agent" }, label);
    assert.equal(frontDoorBannerHref(ENABLED, decision), "https://finite.chat/", label);
  }
});

test("legacy-agent detection treats malformed and N-1 shapes conservatively", () => {
  assert.equal(hasLegacyAgent({ projects: undefined as never, agent_creation_requests: [] }), true);
  assert.equal(hasLegacyAgent({ projects: [], agent_creation_requests: null as never }), true);
  // Abandoned import-bridge rows from an N-1 Core are not product agents.
  assert.equal(
    hasLegacyAgent({ projects: [project("import", true, { import_candidate_id: "candidate" })], agent_creation_requests: [] }),
    false
  );
});

test("a migrated account is redirected even while its legacy agent remains", () => {
  const migrated = me({ projects: [project("p", true)], migrated_at: "2026-10-19T09:00:00Z" });
  assert.deepEqual(frontDoorDecision(ENABLED, account({ me: migrated })), {
    kind: "redirect",
    reason: "migrated",
    url: "https://finite.chat/",
  });
  assert.deepEqual(
    frontDoorDecision(ENABLED, account({ me: migrated, paidSetupPending: true })).kind,
    "redirect"
  );
  // Absent (N-1 Core), null and blank values are not a migration.
  for (const value of [undefined, null, "", "  "]) {
    assert.equal(accountMigrated({ migrated_at: value }), false);
    assert.equal(
      frontDoorDecision(ENABLED, account({ me: me({ projects: [project("p", true)], migrated_at: value }) })).kind,
      "stay"
    );
  }
});

test("operators always keep the legacy dashboard and its admin pages", () => {
  for (const candidate of [
    account({ isAdmin: true }),
    account({ isAdmin: true, me: me({ migrated_at: "2026-10-19T09:00:00Z" }) }),
  ]) {
    assert.deepEqual(frontDoorDecision(ENABLED, candidate), { kind: "stay", reason: "admin" });
  }
});

test("an unavailable or inconsistent account read fails safe to legacy", () => {
  for (const candidate of [
    account({ coreUnavailable: true, me: null }),
    account({ coreUnavailable: true }),
    account({ me: null }),
    account({ workosUserId: null }),
    account({ me: me({ workos_user_id: "user_someone_else" }) }),
  ]) {
    assert.deepEqual(frontDoorDecision(ENABLED, candidate), { kind: "stay", reason: "account-unavailable" });
  }
});

test("a checkout paid before the flip keeps its owner on legacy to finish", () => {
  assert.deepEqual(frontDoorDecision(ENABLED, account({ paidSetupPending: true })), {
    kind: "stay",
    reason: "paid-setup",
  });
});

test("the creation refusal names the configured destination", () => {
  assert.match(frontDoorAgentCreationMovedMessage(ENABLED), /new Finite at finite\.chat\./u);
  assert.match(
    frontDoorAgentCreationMovedMessage({ ...ENABLED, targetUrl: "https://next.finite.chat/" }),
    /at next\.finite\.chat\./u
  );
});

test("the banner dismissal lasts for the browser session only", () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => void values.set(key, value),
  };
  const store = createFrontDoorBannerStore(() => storage);
  let notified = 0;
  const unsubscribe = store.subscribe(() => notified++);
  assert.equal(store.dismissed(), false);
  store.dismiss();
  assert.equal(store.dismissed(), true);
  assert.equal(notified, 1);
  assert.equal(values.get(FRONT_DOOR_BANNER_DISMISSED_KEY), "1");
  // Sign-out clears `finite.`-prefixed session state, so the key is in scope.
  assert.ok(FRONT_DOOR_BANNER_DISMISSED_KEY.startsWith("finite."));
  unsubscribe();
  // Another page in the same session reads the stored dismissal.
  assert.equal(createFrontDoorBannerStore(() => storage).dismissed(), true);

  const refusing = createFrontDoorBannerStore(() => ({
    getItem: () => {
      throw new Error("denied");
    },
    setItem: () => {
      throw new Error("denied");
    },
  }));
  assert.equal(refusing.dismissed(), false);
  refusing.dismiss();
  assert.equal(refusing.dismissed(), true, "privacy-mode storage still hides it for the page");
  assert.equal(createFrontDoorBannerStore(() => null).dismissed(), false);
});

test("only the dashboard page layout can redirect, so no route handler loops", async () => {
  const appDirectory = fileURLToPath(new URL("../app", import.meta.url));
  const files = (await readdir(appDirectory, { recursive: true })).filter((file) =>
    /\.(ts|tsx)$/u.test(file)
  );
  const deciders: string[] = [];
  for (const file of files) {
    const source = await readFile(path.join(appDirectory, file), "utf8");
    if (source.includes("frontDoorDecision")) deciders.push(file.split(path.sep).join("/"));
  }
  // Next.js applies layouts to pages only; route handlers (API routes,
  // /callback, /logout, /site-auth, /google-workspace/*, the Stripe webhook,
  // agent-creation POST/complete) and static assets never render it.
  assert.deepEqual(deciders, ["dashboard/layout.tsx"]);
  for (const route of [
    "callback/route.ts",
    "logout/route.ts",
    "login/route.ts",
    "site-auth/route.ts",
    "google-workspace/start/route.ts",
    "google-workspace/callback/route.ts",
    "api/stripe/webhook/route.ts",
    "health/route.ts",
    "healthz/route.ts",
  ]) {
    assert.ok(files.includes(route.split("/").join(path.sep)), `${route} exists outside the dashboard layout`);
    assert.ok(!route.startsWith("dashboard/"));
  }
  // The dashboard does not serve Core's /internal/* edge routes at all.
  assert.ok(!files.some((file) => file.startsWith(`internal${path.sep}`)));
});
