const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const {
  assertDesktopChatAction,
  dashboardDestination,
  isAllowedUnprivilegedNavigation,
  isDashboardDocumentUrl,
  isGoogleWorkspaceStartUrl,
  normalizeDashboardBaseUrl,
  parseAccountBinding,
  parseDeviceEnrollmentGrant,
  parseDeviceLinkPublicResponse,
  parseLocalDaemonIdentity,
  retryableDashboardStatus,
  shouldExposeLocalChatBridge,
  trustedDashboardIpcFrame,
  trustedDashboardMicrophonePermission,
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

test("device enrollment accepts only the exact encrypted resume grant shape", () => {
  const grant = {
    pairing_session_id: "pairing-session",
    target_device_id: "target-device",
    enrollment_user_id: "user_123",
    enrollment_capability_hex: "ab".repeat(32),
  };
  assert.deepEqual(parseDeviceEnrollmentGrant(grant), grant);
  assert.throws(
    () => parseDeviceEnrollmentGrant({ ...grant, unexpected: true }),
    /enrollment is invalid/u
  );
  assert.throws(
    () => parseDeviceEnrollmentGrant({
      ...grant,
      enrollment_capability_hex: "AB".repeat(32),
    }),
    /enrollment is invalid/u
  );
});

test("only throttling and server failures retry durable dashboard enrollment", () => {
  for (const status of [429, 500, 502, 503, 599]) {
    assert.equal(retryableDashboardStatus(status), true, `${status} retries`);
  }
  for (const status of [0, 200, 400, 401, 403, 404, 409, 410, 600, "503"]) {
    assert.equal(retryableDashboardStatus(status), false, `${status} is terminal`);
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

test("only an explicit development fixture can omit the local chat bridge", () => {
  assert.equal(
    shouldExposeLocalChatBridge({ isPackaged: false, disabledInDevelopment: false }),
    true
  );
  assert.equal(
    shouldExposeLocalChatBridge({ isPackaged: false, disabledInDevelopment: true }),
    false
  );
  assert.equal(
    shouldExposeLocalChatBridge({ isPackaged: true, disabledInDevelopment: true }),
    true
  );
});

test("only dashboard main-frame audio requests receive microphone permission", () => {
  const trustedRequest = {
    permission: "media",
    requestingOrigin: productionDashboard,
    requestingUrl: `${productionDashboard}/dashboard/machines/runtime/chat`,
    securityOrigin: productionDashboard,
    isMainFrame: true,
    mediaTypes: ["audio"],
  };
  assert.equal(
    trustedDashboardMicrophonePermission(trustedRequest, productionDashboard),
    true
  );
  assert.equal(
    trustedDashboardMicrophonePermission(
      { ...trustedRequest, mediaTypes: undefined, mediaType: "audio" },
      productionDashboard
    ),
    true
  );
  assert.equal(
    trustedDashboardMicrophonePermission(
      {
        ...trustedRequest,
        requestingOrigin: `${productionDashboard}/`,
        securityOrigin: `${productionDashboard}/`,
      },
      productionDashboard
    ),
    true
  );
  for (const request of [
    { ...trustedRequest, isMainFrame: false },
    {
      ...trustedRequest,
      requestingOrigin: "https://evil.example",
      requestingUrl: "https://evil.example/dashboard",
      securityOrigin: "https://evil.example",
    },
    { ...trustedRequest, requestingUrl: `${productionDashboard}/login` },
    { ...trustedRequest, mediaTypes: ["video"] },
    { ...trustedRequest, mediaTypes: ["audio", "video"] },
    { ...trustedRequest, mediaTypes: [] },
    { ...trustedRequest, permission: "display-capture" },
  ]) {
    assert.equal(
      trustedDashboardMicrophonePermission(request, productionDashboard),
      false
    );
  }
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
    "SetChatArchived",
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
    pairing_session_id: "pairing-alpha",
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
        bootstrap_manifests: [
          {
            bootstrap_id: "pairing-alpha",
            room_id: "room-one",
            manifest_sha256: "11".repeat(32),
          },
          {
            bootstrap_id: "pairing-alpha",
            room_id: "room-two",
            manifest_sha256: "22".repeat(32),
          },
        ],
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
      bootstrap_manifests: [
        {
          bootstrap_id: "pairing-alpha",
          room_id: "room-one",
          manifest_sha256: "11".repeat(32),
        },
        {
          bootstrap_id: "pairing-alpha",
          room_id: "room-two",
          manifest_sha256: "22".repeat(32),
        },
      ],
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
        bootstrap_manifests: [{
          bootstrap_id: "pairing-alpha",
          room_id: "room-one",
          manifest_sha256: "11".repeat(32),
        }],
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
          bootstrap_manifests: [{
            bootstrap_id: "pairing-alpha",
            room_id: "room-one",
            manifest_sha256: "11".repeat(32),
          }],
        },
        { ...expected, target_device_id: invalidTarget }
      )
    );
  }
  const duplicateManifest = {
    bootstrap_id: "pairing-alpha",
    room_id: "room-one",
    manifest_sha256: "11".repeat(32),
  };
  assert.throws(() =>
    parseDeviceLinkPublicResponse(
      {
        ...expected,
        status: "ready",
        expires_at_unix_seconds: 42,
        room_count: 2,
        active_room_count: 2,
        bootstrap_manifests: [duplicateManifest, duplicateManifest],
      },
      expected
    )
  );
});

test("account and daemon identity projections are exact and fail closed on mismatch", () => {
  const accountId = "a".repeat(64);
  assert.deepEqual(parseAccountBinding({ account_id: accountId }), { account_id: accountId });
  assert.deepEqual(
    parseAccountBinding(
      {
        account_id: accountId,
        local_device: { device_id: "electron-alpha", status: "revoked" },
      },
      "electron-alpha"
    ),
    {
      account_id: accountId,
      local_device: { device_id: "electron-alpha", status: "revoked" },
    }
  );
  assert.throws(() => parseAccountBinding({ account_id: accountId, secret: "no" }));
  assert.throws(() =>
    parseAccountBinding(
      {
        account_id: accountId,
        local_device: { device_id: "electron-other", status: "available" },
      },
      "electron-alpha"
    )
  );
  assert.throws(() =>
    parseAccountBinding(
      {
        account_id: accountId,
        local_device: { device_id: "electron-alpha", status: "surprising" },
      },
      "electron-alpha"
    )
  );
  assert.throws(() => parseAccountBinding({ account_id: "A".repeat(64) }));

  assert.deepEqual(
    parseLocalDaemonIdentity(
      { identity: { account_id: accountId, device_id: "electron-alpha" }, secret: "ignored" },
      accountId,
      "electron-alpha"
    ),
    { account_id: accountId, device_id: "electron-alpha" }
  );
  assert.throws(
    () => parseLocalDaemonIdentity(
      { identity: { account_id: "b".repeat(64), device_id: "electron-alpha" } },
      accountId,
      "electron-alpha"
    ),
    /different Finite account/
  );
  assert.throws(
    () => parseLocalDaemonIdentity(
      { identity: { account_id: accountId, device_id: "electron-other" } },
      accountId,
      "electron-alpha"
    ),
    /different Finite Device identity/
  );
});

test("remote dashboard preload contains only the versioned local-chat bridge", () => {
  const preload = fs.readFileSync(path.join(__dirname, "preload.cjs"), "utf8");
  let exposed;
  vm.runInNewContext(preload, {
    require(moduleName) {
      assert.equal(moduleName, "electron");
      return {
        contextBridge: {
          exposeInMainWorld(name, value) {
            assert.equal(name, "finiteChatDesktop");
            exposed = value;
          },
        },
        ipcRenderer: {
          invoke() {},
          on() {},
          removeListener() {},
        },
      };
    },
    TextEncoder,
  });
  assert.deepEqual([...exposed.capabilities], [
    "local-chat-v1",
    "automatic-device-link-v1",
    "revoked-device-recovery-v1",
    "durable-chat-archive-v1",
  ]);
  assert.deepEqual(Object.keys(exposed).sort(), [
    "attachmentUrl",
    "capabilities",
    "daemonState",
    "dispatchDaemonAction",
    "ensureLocalDevice",
    "onDaemonError",
    "onDaemonGeneration",
    "onDaemonUpdate",
    "onDeviceLinkStatus",
    "recoverLocalDevice",
    "uploadDaemonAttachments",
    "version",
  ].sort());
});

test("daemon bootstrap stays internal and the remote shell does not claim invite deep links", () => {
  const main = fs.readFileSync(path.join(__dirname, "main.cjs"), "utf8");
  const packager = fs.readFileSync(
    path.join(__dirname, "../scripts/package-macos-alpha.mjs"),
    "utf8"
  );
  assert.match(main, /dispatchDaemonAction: dispatchInternalDaemonAction/u);
  assert.doesNotMatch(
    main,
    /currentDashboardAccountBinding\(\)/u,
    "normal startup must not require WorkOS after the encrypted identity commit"
  );
  assert.doesNotMatch(main, /setAsDefaultProtocolClient/u);
  assert.doesNotMatch(packager, /CFBundleURLTypes|CFBundleURLSchemes/u);
});
