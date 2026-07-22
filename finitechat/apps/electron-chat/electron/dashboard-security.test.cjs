const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const {
  assertDesktopChatAction,
  dashboardDestination,
  isAllowedUnprivilegedNavigation,
  isDashboardDocumentUrl,
  isGoogleWorkspaceStartUrl,
  normalizeDashboardBaseUrl,
  parseAccountBinding,
  parseDeviceLinkPublicResponse,
  parseLocalDaemonIdentity,
  trustedDashboardIpcFrame,
} = require("./dashboard-security.cjs");

const productionDashboard = "https://finite.computer";

test("dashboard URL policy permits production HTTPS and loopback development only", () => {
  assert.equal(normalizeDashboardBaseUrl(productionDashboard), productionDashboard);
  assert.equal(normalizeDashboardBaseUrl("http://127.0.0.1:3000"), "http://127.0.0.1:3000");
  assert.equal(
    dashboardDestination(productionDashboard, "/dashboard/machines/runtime/chat?new=1"),
    "https://finite.computer/dashboard/machines/runtime/chat?new=1"
  );
  for (const value of [
    "http://finite.computer",
    "https://user@finite.computer",
    "https://finite.computer/dashboard",
    "file:///tmp/dashboard",
  ]) {
    assert.throws(() => normalizeDashboardBaseUrl(value));
  }
});

test("privileged dashboard documents stay on the exact dashboard subtree", () => {
  assert.equal(isDashboardDocumentUrl("https://finite.computer/dashboard", productionDashboard), true);
  assert.equal(
    isDashboardDocumentUrl("https://finite.computer/dashboard/machines/a/chat", productionDashboard),
    true
  );
  for (const value of [
    "https://finite.computer/",
    "https://finite.computer/login",
    "https://finite.computer.evil.example/dashboard",
    "https://login.workos.com/dashboard",
  ]) {
    assert.equal(isDashboardDocumentUrl(value, productionDashboard), false);
  }
  assert.equal(
    trustedDashboardIpcFrame(
      { frameUrl: "https://finite.computer/dashboard", isMainFrame: true },
      productionDashboard
    ),
    true
  );
  assert.equal(
    trustedDashboardIpcFrame(
      { frameUrl: "https://finite.computer/dashboard", isMainFrame: false },
      productionDashboard
    ),
    false
  );
});

test("unprivileged auth navigation allows HTTPS providers but never active content schemes", () => {
  assert.equal(
    isAllowedUnprivilegedNavigation("https://api.workos.com/sso/authorize", productionDashboard),
    true
  );
  assert.equal(isAllowedUnprivilegedNavigation("https://finite.computer/login", productionDashboard), true);
  assert.equal(isAllowedUnprivilegedNavigation("javascript:alert(1)", productionDashboard), false);
  assert.equal(isAllowedUnprivilegedNavigation("file:///etc/passwd", productionDashboard), false);
});

test("only the exact same-origin Google Workspace start route opens externally", () => {
  assert.equal(
    isGoogleWorkspaceStartUrl(
      "https://finite.computer/google-workspace/start?machineId=runtime-a",
      productionDashboard
    ),
    true
  );
  assert.equal(
    isGoogleWorkspaceStartUrl(
      "http://127.0.0.1:13002/google-workspace/start?machineId=runtime-a",
      "http://127.0.0.1:13002"
    ),
    true
  );
  for (const value of [
    "https://finite.computer/google-workspace/start",
    "https://finite.computer/google-workspace/start?machineId=runtime-a&next=https://evil.example",
    "https://finite.computer/google-workspace/start?machineId=runtime-a#fragment",
    "https://finite.computer/google-workspace/callback?machineId=runtime-a",
    "https://finite.computer.evil.example/google-workspace/start?machineId=runtime-a",
    "javascript:alert(1)",
  ]) {
    assert.equal(isGoogleWorkspaceStartUrl(value, productionDashboard), false);
  }
});

test("desktop chat bridge exposes an exact action allowlist", () => {
  for (const operation of [
    "OpenRoom",
    "CreateTopic",
    "StartTopicChatIntent",
    "SendChatMessage",
    "RefreshDevices",
  ]) {
    const value = { [operation]: operation === "RefreshDevices" ? null : {} };
    assert.equal(assertDesktopChatAction(value).operation, operation);
  }
  for (const value of [
    null,
    [],
    {},
    { RevokeDevice: { account_id: "a", device_id: "b" } },
    { OpenRoom: {}, SendMessage: {} },
  ]) {
    assert.throws(() => assertDesktopChatAction(value));
  }
});

test("device-link responses must match the exact public rendezvous", () => {
  const expected = {
    link_session_id: "link-alpha",
    target_device_id: "electron-alpha",
  };
  assert.deepEqual(
    parseDeviceLinkPublicResponse(
      {
        ...expected,
        status: "ready",
        expires_at_unix_seconds: 42,
        room_count: 2,
        active_room_count: 2,
        account_secret_hex: "ignored-secret-field",
      },
      expected
    ),
    {
      ...expected,
      status: "ready",
      expires_at_unix_seconds: 42,
      room_count: 2,
      active_room_count: 2,
    }
  );
  assert.throws(() =>
    parseDeviceLinkPublicResponse(
      {
        ...expected,
        target_device_id: "electron-other",
        status: "ready",
        expires_at_unix_seconds: 42,
        room_count: 1,
        active_room_count: 1,
      },
      expected
    )
  );
  for (const invalidTarget of [`electron\u200b-alpha`, "é".repeat(129)]) {
    assert.throws(() =>
      parseDeviceLinkPublicResponse(
        {
          ...expected,
          target_device_id: invalidTarget,
          status: "ready",
          expires_at_unix_seconds: 42,
          room_count: 1,
          active_room_count: 1,
        },
        { ...expected, target_device_id: invalidTarget }
      )
    );
  }
});

test("account and daemon identity projections are exact and fail closed on mismatch", () => {
  const accountId = "a".repeat(64);
  assert.deepEqual(parseAccountBinding({ account_id: accountId }), { account_id: accountId });
  assert.throws(() => parseAccountBinding({ account_id: accountId, secret: "no" }));
  assert.throws(() => parseAccountBinding({ account_id: "A".repeat(64) }));

  assert.deepEqual(
    parseLocalDaemonIdentity(
      { identity: { account_id: accountId, device_id: "electron-alpha" }, secret: "ignored" },
      accountId
    ),
    { account_id: accountId, device_id: "electron-alpha" }
  );
  assert.throws(
    () => parseLocalDaemonIdentity(
      { identity: { account_id: "b".repeat(64), device_id: "electron-alpha" } },
      accountId
    ),
    /different Finite account/
  );
});

test("remote dashboard preload contains only the versioned local-chat bridge", () => {
  const preload = fs.readFileSync(path.join(__dirname, "preload.cjs"), "utf8");
  for (const capability of [
    "ensureLocalDevice",
    "daemonState",
    "dispatchDaemonAction",
    "uploadDaemonAttachments",
    "attachmentUrl",
    "onDaemonUpdate",
    "onDaemonGeneration",
    "onDaemonError",
    "onDeviceLinkStatus",
  ]) {
    assert.match(preload, new RegExp(`\\b${capability}\\b`));
  }
  for (const forbidden of [
    "clearAccountSecret",
    "openDeviceLinkApproval",
    "copyText",
    "completeOnboarding",
    "consumePendingTargetUrl",
    "daemonConnection",
  ]) {
    assert.doesNotMatch(preload, new RegExp(`\\b${forbidden}\\b`));
  }
});

test("daemon bootstrap stays internal and the remote shell does not claim invite deep links", () => {
  const main = fs.readFileSync(path.join(__dirname, "main.cjs"), "utf8");
  const packager = fs.readFileSync(
    path.join(__dirname, "../scripts/package-macos-alpha.mjs"),
    "utf8"
  );
  assert.match(main, /dispatchDaemonAction: dispatchInternalDaemonAction/u);
  assert.doesNotMatch(main, /setAsDefaultProtocolClient/u);
  assert.doesNotMatch(packager, /CFBundleURLTypes|CFBundleURLSchemes/u);
});
