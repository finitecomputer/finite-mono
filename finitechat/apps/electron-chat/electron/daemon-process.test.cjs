const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { PassThrough } = require("node:stream");
const test = require("node:test");

const {
  attachmentActionUsesBinaryTransport,
  attachmentUploadForm,
  forwardAttachmentUpload,
  validateAttachmentByteLengths,
} = require("./attachment-upload.cjs");
const {
  attachmentMediaUrl,
  parseAttachmentMediaUrl,
} = require("./attachment-media.cjs");

const {
  DEFAULT_READY_TIMEOUT_MS,
  DeviceIdentityStore,
  DaemonUpdateRelay,
  DaemonSupervisor,
  DeviceLinkSupervisor,
  archiveRevokedDeviceProfile,
  daemonRequestVersionMatches,
  deviceLinkFailureMessage,
  legacyHostnameDeviceId,
  loadOrCreateDeviceId,
  markDeviceProfileInitialized,
  parseReadyRecord,
  parseDeviceLinkReadyRecord,
  parseDeviceLinkSecretRecord,
  parseDeviceIdentityEnvelope,
  parseDeviceLinkBootstrapError,
  removeDeprecatedDeviceLinkSetting,
  resolveDaemonBinary,
  startDaemonRuntime,
  startupDocument,
} = require("./daemon-process.cjs");

test("the default daemon readiness budget covers complete-history cold starts", () => {
  assert.ok(DEFAULT_READY_TIMEOUT_MS >= 60_000);
});

function temporaryDirectory() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "finitechat-electron-process-"));
}

function testSafeStorage() {
  return {
    isEncryptionAvailable: () => true,
    getSelectedStorageBackend: () => "keychain",
    encryptString: (value) => Buffer.from(
      Buffer.from(value, "utf8").toString("base64"),
      "utf8"
    ),
    decryptString: (value) => Buffer.from(
      Buffer.from(value).toString("utf8"),
      "base64"
    ).toString("utf8"),
  };
}

function testDeviceIdentityStore(root) {
  return new DeviceIdentityStore({
    identityPath: path.join(root, "device-identity.safe"),
    safeStorage: testSafeStorage(),
  });
}

function emitDeviceLinkReady(child) {
  child.stdout.write(
    `${JSON.stringify({
      event: "pairing_ready",
      pairing_session_id: "pairing-public-test",
      target_device_id: "electron-test-device",
    })}\n`
  );
}

function privateDeviceLinkResult(secretCharacter) {
  return {
    account_secret: secretCharacter.repeat(64),
    enrollment_user_id: "user_test",
    enrollment_capability_hex: "ab".repeat(32),
  };
}

function identityEnvelope(secretCharacter = "d", overrides = {}) {
  return {
    version: 1,
    account_secret: secretCharacter.repeat(64),
    expected_account_id: "12".repeat(32),
    expected_device_id: "electron-test-device",
    pending_enrollment: {
      pairing_session_id: "pairing-public-test",
      target_device_id: "electron-test-device",
      enrollment_user_id: "user_test",
      enrollment_capability_hex: "ab".repeat(32),
    },
    ...overrides,
  };
}

test("attachment upload form preserves binary views and scoped multipart fields", async () => {
  const first = Uint8Array.from([0, 255, 13, 10]).buffer;
  const secondBacking = Uint8Array.from([9, 8, 7, 6]);
  const form = attachmentUploadForm({
    room_id: " room-test ",
    topic_id: "topic-test",
    chat_id: "chat-test",
    caption: " binary proof ",
    reply_to_message_id: "message-parent",
    files: [
      { filename: "folder/proof.bin", mime_type: "APPLICATION/OCTET-STREAM", bytes: first },
      { filename: "second.txt", mime_type: "text/plain", bytes: secondBacking.subarray(1, 3) },
    ],
  });

  assert.equal(form.get("room_id"), "room-test");
  assert.equal(form.get("topic_id"), "topic-test");
  assert.equal(form.get("chat_id"), "chat-test");
  assert.equal(form.get("caption"), "binary proof");
  assert.equal(form.get("reply_to_message_id"), "message-parent");
  const files = form.getAll("files");
  assert.equal(files.length, 2);
  assert.equal(files[0].name, "proof.bin");
  assert.equal(files[0].type, "application/octet-stream");
  assert.deepEqual(Buffer.from(await files[0].arrayBuffer()), Buffer.from([0, 255, 13, 10]));
  assert.deepEqual(Buffer.from(await files[1].arrayBuffer()), Buffer.from([8, 7]));
});

test("attachment upload bounds are checked numerically without giant buffers", () => {
  const fileLimit = 32 * 1024 * 1024;
  const totalLimit = 64 * 1024 * 1024;
  assert.equal(validateAttachmentByteLengths([fileLimit]), fileLimit);
  assert.equal(validateAttachmentByteLengths([fileLimit, fileLimit]), totalLimit);
  assert.throws(() => validateAttachmentByteLengths([fileLimit + 1]), /between 1/);
  assert.throws(() => validateAttachmentByteLengths([fileLimit, fileLimit, 1]), /total at most/);
  assert.throws(() => validateAttachmentByteLengths(Array(9).fill(1)), /between 1 and 8 files/);
  assert.throws(() => validateAttachmentByteLengths([0]), /between 1/);
});

test("attachment transport forwards one validated FormData and owns attachment actions", async () => {
  const upload = {
    room_id: "room-test",
    caption: "",
    files: [{ filename: "proof.bin", mime_type: "application/octet-stream", bytes: Uint8Array.of(1) }],
  };
  let forwarded = null;
  const result = await forwardAttachmentUpload(upload, async (form) => {
    forwarded = form;
    return { status: "ok" };
  });
  assert.equal(forwarded.get("room_id"), "room-test");
  assert.deepEqual(result, { status: "ok" });
  assert.equal(attachmentActionUsesBinaryTransport({ SendAttachments: {} }), true);
  assert.equal(attachmentActionUsesBinaryTransport({ SendChatAttachments: {} }), true);
  assert.equal(attachmentActionUsesBinaryTransport({ SendMessage: {} }), false);
});

test("attachment media URLs contain only three opaque identifiers and reject traversal", () => {
  const url = attachmentMediaUrl({
    room_id: "room one",
    message_id: "message:test",
    attachment_id: "sha256-deadbeef",
  });
  assert.deepEqual(parseAttachmentMediaUrl(url), {
    room_id: "room one",
    message_id: "message:test",
    attachment_id: "sha256-deadbeef",
  });
  for (const invalid of [
    "finitechat-media://attachment/../message/attachment",
    "finitechat-media://attachment/room/%2Fetc/attachment",
    "finitechat-media://attachment/room//attachment",
    "finitechat-media://attachment/room/message/%5Coutside",
    "finitechat-media://other/room/message/attachment",
    "finitechat-media://attachment/room/message/attachment?path=/tmp/secret",
  ]) {
    assert.throws(() => parseAttachmentMediaUrl(invalid), /invalid/);
  }
});

test("ready records accept only dynamic loopback HTTP addresses", () => {
  assert.equal(parseReadyRecord('{"event":"ready","url":"http://127.0.0.1:43123"}'), "http://127.0.0.1:43123");
  assert.equal(parseReadyRecord('{"event":"ready","url":"http://[::1]:43123"}'), "http://[::1]:43123");
  assert.throws(
    () => parseReadyRecord('{"event":"ready","url":"http://0.0.0.0:43123"}'),
    /unsafe ready address/
  );
  assert.throws(
    () => parseReadyRecord('{"event":"ready","url":"https://127.0.0.1:43123"}'),
    /unsafe ready address/
  );
  assert.throws(() => parseReadyRecord("not-json"), /invalid ready record/);
});

test("fresh Device ids are random, persisted, and independent of hostname", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const settingsFile = path.join(root, "desktop-settings.json");
  const daemonDataDirectory = path.join(root, "finitechatd");
  const first = loadOrCreateDeviceId({
    settingsFile,
    daemonDataDirectory,
    hostname: "pauls-mac.local",
    randomUUID: () => "11111111-2222-4333-8444-555555555555",
  });
  const second = loadOrCreateDeviceId({
    settingsFile,
    daemonDataDirectory,
    hostname: "renamed-mac.local",
    randomUUID: () => "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
  });
  assert.equal(first, "electron-11111111-2222-4333-8444-555555555555");
  assert.equal(second, first);
  assert.equal(JSON.parse(fs.readFileSync(settingsFile, "utf8")).deviceId, first);
  assert.deepEqual(
    JSON.parse(fs.readFileSync(path.join(daemonDataDirectory, "device-identity.json"), "utf8")),
    { version: 1, deviceId: first, initialized: false }
  );
});

test("deleting a cryptographic profile rotates a stale desktop Device id", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const settingsFile = path.join(root, "desktop-settings.json");
  const daemonDataDirectory = path.join(root, "finitechatd");
  const first = loadOrCreateDeviceId({
    settingsFile,
    daemonDataDirectory,
    hostname: "host",
    randomUUID: () => "11111111-2222-4333-8444-555555555555",
  });
  fs.writeFileSync(path.join(daemonDataDirectory, "client.sqlite3"), "cryptographic-state");
  fs.rmSync(daemonDataDirectory, { recursive: true });

  const replacement = loadOrCreateDeviceId({
    settingsFile,
    daemonDataDirectory,
    hostname: "host",
    randomUUID: () => "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
  });
  assert.notEqual(replacement, first);
  assert.equal(replacement, "electron-aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee");
  assert.equal(JSON.parse(fs.readFileSync(settingsFile, "utf8")).deviceId, replacement);
  assert.equal(
    JSON.parse(fs.readFileSync(path.join(daemonDataDirectory, "device-identity.json"), "utf8")).deviceId,
    replacement
  );
});

test("a persisted cryptographic profile restores its Device id mirror", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const settingsFile = path.join(root, "desktop-settings.json");
  const daemonDataDirectory = path.join(root, "finitechatd");
  const deviceId = loadOrCreateDeviceId({
    settingsFile,
    daemonDataDirectory,
    hostname: "host",
    randomUUID: () => "11111111-2222-4333-8444-555555555555",
  });
  fs.rmSync(settingsFile);

  assert.equal(
    loadOrCreateDeviceId({
      settingsFile,
      daemonDataDirectory,
      hostname: "renamed-host",
      randomUUID: () => {
        throw new Error("persisted profile must not rotate");
      },
    }),
    deviceId
  );
  assert.equal(JSON.parse(fs.readFileSync(settingsFile, "utf8")).deviceId, deviceId);
});

test("rebuilding an initialized local store rotates its Device generation", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const settingsFile = path.join(root, "desktop-settings.json");
  const daemonDataDirectory = path.join(root, "finitechatd");
  const first = loadOrCreateDeviceId({
    settingsFile,
    daemonDataDirectory,
    hostname: "host",
    randomUUID: () => "11111111-2222-4333-8444-555555555555",
  });
  fs.writeFileSync(path.join(daemonDataDirectory, "client.sqlite3"), "cryptographic-state");
  markDeviceProfileInitialized({ daemonDataDirectory, deviceId: first });
  fs.writeFileSync(path.join(daemonDataDirectory, `account-secret.${first}.safe`), "encrypted-secret");
  fs.rmSync(path.join(daemonDataDirectory, "client.sqlite3"));

  const replacement = loadOrCreateDeviceId({
    settingsFile,
    daemonDataDirectory,
    hostname: "host",
    randomUUID: () => "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
  });
  assert.equal(replacement, "electron-aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee");
  assert.notEqual(replacement, first);
  assert.equal(fs.existsSync(path.join(daemonDataDirectory, `account-secret.${first}.safe`)), true);
  assert.equal(
    fs.existsSync(path.join(daemonDataDirectory, `account-secret.${replacement}.safe`)),
    false
  );
  assert.deepEqual(
    JSON.parse(fs.readFileSync(path.join(daemonDataDirectory, "device-identity.json"), "utf8")),
    { version: 1, deviceId: replacement, initialized: false }
  );
});

test("a pre-alpha data directory pins the legacy hostname Device id once", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const settingsFile = path.join(root, "desktop-settings.json");
  const daemonDataDirectory = path.join(root, "finitechatd");
  fs.mkdirSync(daemonDataDirectory, { recursive: true });
  fs.writeFileSync(path.join(daemonDataDirectory, "client.sqlite3"), "old-data");

  const migrated = loadOrCreateDeviceId({
    settingsFile,
    daemonDataDirectory,
    hostname: "Paul's Mac / alpha",
    randomUUID: () => {
      throw new Error("legacy migration must not create a different Device");
    },
  });
  assert.equal(migrated, legacyHostnameDeviceId("Paul's Mac / alpha"));
  assert.equal(
    loadOrCreateDeviceId({
      settingsFile,
      daemonDataDirectory,
      hostname: "new-hostname",
      randomUUID: () => "unused",
    }),
    migrated
  );
  assert.equal(
    JSON.parse(fs.readFileSync(path.join(daemonDataDirectory, "device-identity.json"), "utf8"))
      .initialized,
    true
  );
});

test("revoked Device recovery archives local cryptographic state and creates a fresh identity boundary", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const userDataDirectory = path.join(root, "user-data");
  const daemonDataDirectory = path.join(userDataDirectory, "finitechatd");
  const settingsFile = path.join(userDataDirectory, "desktop-settings.json");
  const deviceId = "electron-revoked-alpha";
  const secretFile = path.join(
    daemonDataDirectory,
    `account-secret.${deviceId}.safe`
  );
  const leakedCapability = "cd".repeat(32);
  fs.mkdirSync(daemonDataDirectory, { recursive: true });
  fs.writeFileSync(path.join(daemonDataDirectory, "client.sqlite3"), "encrypted-state");
  fs.writeFileSync(secretFile, "encrypted-identity-envelope");
  fs.writeFileSync(settingsFile, `${JSON.stringify({
    deviceId,
    pendingDeviceLink: {
      pairing_session_id: "old-link",
      target_device_id: deviceId,
      enrollment_user_id: "user_old",
      enrollment_capability_hex: leakedCapability,
    },
    dashboardPreference: "keep-me",
  })}\n`);

  const archived = archiveRevokedDeviceProfile({
    userDataDirectory,
    daemonDataDirectory,
    settingsFile,
    secretFile,
    deviceId,
    now: new Date("2026-07-22T19:00:00.000Z"),
    randomUUID: () => "11111111-2222-4333-8444-555555555555",
  });

  assert.equal(fs.existsSync(daemonDataDirectory), false);
  assert.equal(fs.existsSync(secretFile), false);
  assert.equal(
    fs.readFileSync(path.join(archived.backupDirectory, "finitechatd", "client.sqlite3"), "utf8"),
    "encrypted-state"
  );
  assert.equal(
    fs.readFileSync(
      path.join(
        archived.backupDirectory,
        "finitechatd",
        `account-secret.${deviceId}.safe`
      ),
      "utf8"
    ),
    "encrypted-identity-envelope"
  );
  assert.deepEqual(JSON.parse(fs.readFileSync(settingsFile, "utf8")), {
    dashboardPreference: "keep-me",
  });
  assert.deepEqual(
    JSON.parse(
      fs.readFileSync(
        path.join(archived.backupDirectory, "desktop-settings.json"),
        "utf8"
      )
    ),
    {
      deviceId,
      dashboardPreference: "keep-me",
    }
  );
  assert.doesNotMatch(
    fs.readFileSync(
      path.join(archived.backupDirectory, "desktop-settings.json"),
      "utf8"
    ),
    new RegExp(leakedCapability)
  );
  assert.deepEqual(
    JSON.parse(fs.readFileSync(path.join(archived.backupDirectory, "recovery.json"), "utf8")),
    {
      version: 1,
      reason: "device_revoked",
      device_id: deviceId,
      archived_at: "2026-07-22T19:00:00.000Z",
    }
  );
  assert.equal(
    loadOrCreateDeviceId({
      settingsFile,
      daemonDataDirectory,
      hostname: "same-hostname",
      randomUUID: () => "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
    }),
    "electron-aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
  );
});

test("revoked Device archive rolls the original profile back after a partial failure", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const userDataDirectory = path.join(root, "user-data");
  const daemonDataDirectory = path.join(userDataDirectory, "finitechatd");
  const settingsFile = path.join(userDataDirectory, "desktop-settings.json");
  const deviceId = "electron-revoked-alpha";
  const secretFile = path.join(
    daemonDataDirectory,
    `account-secret.${deviceId}.safe`
  );
  fs.mkdirSync(daemonDataDirectory, { recursive: true });
  fs.writeFileSync(path.join(daemonDataDirectory, "client.sqlite3"), "encrypted-state");
  fs.writeFileSync(secretFile, "encrypted-account-secret");
  fs.writeFileSync(settingsFile, `${JSON.stringify({ deviceId })}\n`);
  const failingFileSystem = {
    ...fs,
    writeFileSync(filePath, ...args) {
      if (/recovery\.json\.[0-9]+\.tmp$/u.test(String(filePath))) {
        throw new Error("synthetic metadata failure");
      }
      return fs.writeFileSync(filePath, ...args);
    },
  };

  assert.throws(
    () => archiveRevokedDeviceProfile({
      userDataDirectory,
      daemonDataDirectory,
      settingsFile,
      secretFile,
      deviceId,
      now: new Date("2026-07-22T19:00:00.000Z"),
      randomUUID: () => "11111111-2222-4333-8444-555555555555",
      fileSystem: failingFileSystem,
    }),
    /synthetic metadata failure/
  );
  assert.equal(fs.readFileSync(path.join(daemonDataDirectory, "client.sqlite3"), "utf8"), "encrypted-state");
  assert.equal(fs.readFileSync(secretFile, "utf8"), "encrypted-account-secret");
  assert.deepEqual(JSON.parse(fs.readFileSync(settingsFile, "utf8")), { deviceId });
});

test("invalid persisted Device settings fail closed instead of forking local state", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const settingsFile = path.join(root, "desktop-settings.json");
  fs.writeFileSync(settingsFile, '{"deviceId":"not a valid id"}\n');
  assert.throws(
    () =>
      loadOrCreateDeviceId({
        settingsFile,
        daemonDataDirectory: path.join(root, "finitechatd"),
        hostname: "host",
      }),
    /invalid Device id/
  );
});

test("binary resolution accepts only explicit dev or packaged finitechatd files", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const explicit = path.join(root, "custom-finitechatd");
  fs.writeFileSync(explicit, "binary");
  assert.equal(
    resolveDaemonBinary({ explicitPath: explicit, isPackaged: false, resourcesPath: root }),
    explicit
  );

  const resources = path.join(root, "resources");
  fs.mkdirSync(resources);
  const packaged = path.join(resources, process.platform === "win32" ? "finitechatd.exe" : "finitechatd");
  fs.writeFileSync(packaged, "binary");
  assert.equal(resolveDaemonBinary({ isPackaged: true, resourcesPath: resources }), packaged);
  assert.throws(
    () => resolveDaemonBinary({ isPackaged: false, resourcesPath: resources }),
    /FINITECHAT_DAEMON_BINARY/
  );
});

test("startup documents are bounded JSON and never require argv secrets", () => {
  assert.deepEqual(JSON.parse(startupDocument("a".repeat(64), "b".repeat(64))), {
    auth_token: "a".repeat(64),
    account_secret: "b".repeat(64),
  });
  assert.throws(() => startupDocument("a".repeat(64), "b".repeat(3_000)), /too large/);
});

test("device-link public and private records are narrow and independently validated", () => {
  const ready = parseDeviceLinkReadyRecord(
    JSON.stringify({
      event: "pairing_ready",
      pairing_session_id: "pairing-public-test",
      target_device_id: "electron-test-device",
    })
  );
  assert.deepEqual(ready, {
    pairing_session_id: "pairing-public-test",
    target_device_id: "electron-test-device",
  });
  assert.deepEqual(
    parseDeviceLinkSecretRecord(JSON.stringify(privateDeviceLinkResult("c"))),
    {
      accountSecret: "c".repeat(64),
      enrollmentUserId: "user_test",
      enrollmentCapabilityHex: "ab".repeat(32),
    }
  );
  assert.throws(
    () => parseDeviceLinkReadyRecord(
      '{"event":"pairing_ready","pairing_session_id":"pairing-public-test","target_device_id":"electron-test-device","unexpected":"value"}'
    ),
    /invalid status record/
  );
  assert.throws(
    () => parseDeviceLinkSecretRecord(JSON.stringify({
      ...privateDeviceLinkResult("c"),
      account_secret: "not-secret-material",
    })),
    /invalid private result/
  );
});

test("identity envelope is exact, encrypted as one record, and rewrites atomically", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const store = testDeviceIdentityStore(root);
  const identity = identityEnvelope();
  const sensitiveValues = [
    identity.account_secret,
    identity.expected_account_id,
    identity.pending_enrollment.enrollment_capability_hex,
  ];

  assert.deepEqual(parseDeviceIdentityEnvelope(identity), identity);
  assert.throws(
    () => parseDeviceIdentityEnvelope({ ...identity, unexpected: true }),
    /stored identity is invalid/u
  );

  store.writeProvisional(identity);
  assert.equal(store.read(), null, "a pre-rename crash has no active identity");
  const provisionalBytes = fs.readFileSync(store.provisionalPath).toString("utf8");
  for (const sensitive of sensitiveValues) {
    assert.doesNotMatch(provisionalBytes, new RegExp(sensitive));
  }
  store.promoteProvisional();
  assert.deepEqual(store.read(), identity);

  const completed = { ...identity, pending_enrollment: null };
  store.write(completed);
  assert.deepEqual(store.read(), completed);
  const activeBytes = fs.readFileSync(store.identityPath).toString("utf8");
  for (const sensitive of sensitiveValues) {
    assert.doesNotMatch(activeBytes, new RegExp(sensitive));
  }
});

test("startup removes a pre-release plaintext enrollment capability", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const settingsFile = path.join(root, "desktop-settings.json");
  const capability = "ef".repeat(32);
  fs.writeFileSync(settingsFile, `${JSON.stringify({
    deviceId: "electron-test-device",
    pendingDeviceLink: {
      pairing_session_id: "old-pairing",
      target_device_id: "electron-test-device",
      enrollment_user_id: "user_old",
      enrollment_capability_hex: capability,
    },
    dashboardPreference: "preserved",
  })}\n`);

  assert.equal(
    removeDeprecatedDeviceLinkSetting({ settingsFile }),
    true
  );
  const plaintext = fs.readFileSync(settingsFile, "utf8");
  assert.deepEqual(JSON.parse(plaintext), {
    deviceId: "electron-test-device",
    dashboardPreference: "preserved",
  });
  assert.doesNotMatch(plaintext, new RegExp(capability));
  assert.equal(
    removeDeprecatedDeviceLinkSetting({ settingsFile }),
    false,
    "cleanup is idempotent"
  );
});

test("failed enrollment-clear rename preserves the complete resumable identity", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const initial = testDeviceIdentityStore(root);
  const identity = identityEnvelope();
  initial.write(identity);
  const failingFileSystem = {
    ...fs,
    renameSync(source, destination) {
      if (destination === initial.identityPath && /\.tmp$/u.test(source)) {
        throw new Error("synthetic crash before identity rename");
      }
      return fs.renameSync(source, destination);
    },
  };
  const restarted = new DeviceIdentityStore({
    identityPath: initial.identityPath,
    safeStorage: testSafeStorage(),
    fileSystem: failingFileSystem,
  });

  assert.throws(
    () => restarted.write({ ...identity, pending_enrollment: null }),
    /synthetic crash/
  );
  assert.deepEqual(initial.read(), identity);
  assert.equal(
    fs.readdirSync(root).some((name) => name.endsWith(".tmp")),
    false
  );
});

test("corrupt encrypted identity fails closed instead of looking unpaired", (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const store = testDeviceIdentityStore(root);
  fs.writeFileSync(store.identityPath, Buffer.from("not-an-envelope"));
  assert.throws(() => store.read(), /stored identity is invalid/u);
});

test("device-link child failures use a bounded exact allowlist with fixed renderer copy", () => {
  const knownFailures = new Map([
    ["invalid device-link configuration", "FINITECHAT_DEVICE_LINK_INVALID_CONFIGURATION"],
    ["device-link entropy generation failed", "FINITECHAT_DEVICE_LINK_ENTROPY"],
    ["device-link server request failed", "FINITECHAT_DEVICE_LINK_REQUEST"],
    ["device-link server returned an invalid response", "FINITECHAT_DEVICE_LINK_INVALID_RESPONSE"],
    ["device-link request expired", "FINITECHAT_DEVICE_LINK_EXPIRED"],
    ["device-link payload failed authentication", "FINITECHAT_DEVICE_LINK_PAYLOAD_REJECTED"],
    ["device-link result pipe failed", "FINITECHAT_DEVICE_LINK_RESULT_PIPE"],
  ]);
  for (const [line, code] of knownFailures) {
    assert.equal(parseDeviceLinkBootstrapError(line)?.code, code);
  }

  const payloadRejected = parseDeviceLinkBootstrapError("device-link payload failed authentication");
  assert.equal(payloadRejected.code, "FINITECHAT_DEVICE_LINK_PAYLOAD_REJECTED");
  assert.equal(
    deviceLinkFailureMessage(payloadRejected),
    "The approved device-link payload did not match this link. Start a new link to try again."
  );

  const serverStatus = parseDeviceLinkBootstrapError("device-link server rejected the request (502)");
  assert.equal(serverStatus.code, "FINITECHAT_DEVICE_LINK_SERVER_STATUS");
  assert.doesNotMatch(deviceLinkFailureMessage(serverStatus), /502/);

  for (const unsafe of [
    "device-link payload failed authentication: link-public-secret",
    '{"server_body":"private response"}',
    "device-link server rejected the request (1234)",
    "x".repeat(4 * 1024 + 1),
  ]) {
    assert.equal(parseDeviceLinkBootstrapError(unsafe), null);
  }
  assert.equal(
    deviceLinkFailureMessage({ code: "toString" }),
    "This desktop could not be linked. Start a new link to try again."
  );
});

class FakeChild extends EventEmitter {
  constructor() {
    super();
    this.stdin = new PassThrough();
    this.stdout = new PassThrough();
    this.stderr = new PassThrough();
    this.exitCode = null;
    this.signalCode = null;
    this.kills = [];
  }

  kill(signal = "SIGTERM") {
    this.kills.push(signal);
    this.signalCode = signal;
    queueMicrotask(() => this.emit("exit", null, signal));
    return true;
  }

  exit(code = 0) {
    this.exitCode = code;
    this.emit("exit", code, null);
  }
}

class FakeLinkChild extends EventEmitter {
  constructor() {
    super();
    this.stdout = new PassThrough();
    this.stderr = new PassThrough();
    this.stdio = [null, this.stdout, this.stderr, new PassThrough(), new PassThrough(), new PassThrough()];
    this.exitCode = null;
    this.signalCode = null;
  }

  kill(signal = "SIGTERM") {
    this.signalCode = signal;
    queueMicrotask(() => {
      this.emit("exit", null, signal);
      this.emit("close", null, signal);
    });
    return true;
  }

  exit(code = 0, { close = true } = {}) {
    this.exitCode = code;
    this.emit("exit", code, null);
    if (close) {
      this.emit("close", code, null);
    }
  }

  close(code = this.exitCode, signal = this.signalCode) {
    this.emit("close", code, signal);
  }
}

test("supervisor authenticates a dynamic daemon and rotates its bearer on restart", async () => {
  const children = [];
  const startupDocuments = [];
  const requestAuthorizations = [];
  let tokenSeed = 1;
  const spawnProcess = (_binary, args, options) => {
    assert.deepEqual(args.slice(0, 2), ["--bind", "127.0.0.1:0"]);
    assert.deepEqual(options.stdio, ["pipe", "pipe", "pipe"]);
    const child = new FakeChild();
    children.push(child);
    let stdin = "";
    child.stdin.on("data", (chunk) => {
      stdin += chunk.toString();
    });
    child.stdin.on("end", () => startupDocuments.push(JSON.parse(stdin)));
    queueMicrotask(() => {
      child.stdout.write(`{"event":"ready","url":"http://127.0.0.1:${44000 + children.length}"}\n`);
    });
    return child;
  };
  const fetchImpl = async (_url, init) => {
    requestAuthorizations.push(new Headers(init.headers).get("authorization"));
    return new Response('{"status":"ok"}', { status: 200, headers: { "content-type": "application/json" } });
  };
  const supervisor = new DaemonSupervisor({
    spawnProcess,
    binaryPath: "/tmp/finitechatd",
    args: ["--bind", "127.0.0.1:0"],
    cwd: "/tmp",
    accountSecret: "account-material",
    fetchImpl,
    randomBytes: () => Buffer.alloc(32, tokenSeed++),
  });

  await Promise.all([supervisor.start(), supervisor.start()]);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(children.length, 1);
  assert.equal(startupDocuments[0].account_secret, "account-material");
  assert.equal(requestAuthorizations[0], `Bearer ${"01".repeat(32)}`);

  await supervisor.restart({ accountSecret: null });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(children.length, 2);
  assert.equal(startupDocuments[1].account_secret, undefined);
  assert.equal(requestAuthorizations[1], `Bearer ${"02".repeat(32)}`);
  assert.equal(children[0].kills[0], "SIGTERM");
  await supervisor.stop();
});

test("unexpected ready-process exits revoke the in-memory connection", async () => {
  const child = new FakeChild();
  let failure = null;
  const supervisor = new DaemonSupervisor({
    spawnProcess: () => {
      queueMicrotask(() => child.stdout.write('{"event":"ready","url":"http://127.0.0.1:44001"}\n'));
      return child;
    },
    binaryPath: "/tmp/finitechatd",
    args: [],
    cwd: "/tmp",
    fetchImpl: async () => new Response("{}", { status: 200 }),
    onUnexpectedExit: (error) => {
      failure = error;
    },
  });
  await supervisor.start();
  child.exit(7);
  assert.match(failure.message, /stopped unexpectedly/);
  await assert.rejects(() => supervisor.requestJson("/v1/app/state"), /unavailable/);
});

test("device link stores the fd3 secret before fd4 confirmation and clean completion", async () => {
  const child = new FakeLinkChild();
  let spawnArgs = null;
  let spawnOptions = null;
  const storedIdentities = [];
  let confirmation = "";
  child.stdio[4].on("data", (chunk) => {
    confirmation += chunk.toString();
  });
  const link = new DeviceLinkSupervisor({
    spawnProcess: (_binary, args, options) => {
      spawnArgs = args;
      spawnOptions = options;
      return child;
    },
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeIdentityEnvelope: async (accountSecret, pendingEnrollment) => {
      storedIdentities.push({ accountSecret, pendingEnrollment });
    },
  });
  const readyPromise = link.begin();
  const durable = link.durable;
  const completion = link.completion;
  child.stdout.write(
    `${JSON.stringify({
      event: "pairing_ready",
      pairing_session_id: "pairing-public-test",
      target_device_id: "electron-test-device",
    })}\n`
  );
  const ready = await readyPromise;
  assert.equal(ready.target_device_id, "electron-test-device");
  assert.deepEqual(spawnOptions.stdio, ["ignore", "pipe", "pipe", "pipe", "pipe", "pipe"]);
  assert.deepEqual(spawnArgs, [
    "link",
    "--server-url",
    "https://chat.finite.computer",
    "--device-id",
    "electron-test-device",
    "--result-fd",
    "3",
    "--confirm-fd",
    "4",
    "--descriptor-fd",
    "5",
  ]);

  link.acceptSourceDescriptor({
    version: 1,
    source_public_key: "a".repeat(64),
    session_secret_hex: "b".repeat(64),
    expires_at_unix_seconds: 42,
  });
  child.stdio[3].write(`${JSON.stringify(privateDeviceLinkResult("d"))}\n`);
  const storedEnrollment = await durable;
  assert.equal(confirmation, "stored\n");
  let completionSettled = false;
  completion.finally(() => {
    completionSettled = true;
  });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(
    completionSettled,
    false,
    "durable storage succeeds before NIP Complete or child exit"
  );
  assert.deepEqual(storedIdentities, [{
    accountSecret: "d".repeat(64),
    pendingEnrollment: {
      pairing_session_id: "pairing-public-test",
      target_device_id: "electron-test-device",
      enrollment_user_id: "user_test",
      enrollment_capability_hex: "ab".repeat(32),
    },
  }]);
  assert.deepEqual(storedEnrollment, {
    pairing_session_id: "pairing-public-test",
    target_device_id: "electron-test-device",
    enrollment_user_id: "user_test",
    enrollment_capability_hex: "ab".repeat(32),
  });
  child.stdout.write('{"event":"linked"}\n');
  child.exit(0);
  assert.deepEqual(await completion, storedEnrollment);
});

test("device link propagates an allowlisted payload rejection without reflecting other stderr", async () => {
  const child = new FakeLinkChild();
  const link = new DeviceLinkSupervisor({
    spawnProcess: () => child,
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeIdentityEnvelope: async () => {},
  });
  const readyPromise = link.begin();
  const completion = link.completion;
  emitDeviceLinkReady(child);
  await readyPromise;

  const privateStderr = "link-public-private-token server-body-private";
  child.stderr.write(`${privateStderr}\n`);
  child.stderr.write("device-link payload failed authentication\n");
  child.exit(1);

  await assert.rejects(completion, (error) => {
    assert.equal(error.code, "FINITECHAT_DEVICE_LINK_PAYLOAD_REJECTED");
    assert.equal(
      error.message,
      "The approved device-link payload did not match this link. Start a new link to try again."
    );
    assert.doesNotMatch(error.message, new RegExp(privateStderr));
    return true;
  });
});

test("device link keeps unknown child stderr behind the generic renderer failure", async () => {
  const child = new FakeLinkChild();
  const link = new DeviceLinkSupervisor({
    spawnProcess: () => child,
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeIdentityEnvelope: async () => {},
  });
  const readyPromise = link.begin();
  const completion = link.completion;
  emitDeviceLinkReady(child);
  await readyPromise;

  const privateStderr = "server said link-public-private-token in a private body";
  child.stderr.write("x".repeat(4 * 1024 + 1));
  assert.equal(child.stderr.readableFlowing, true);
  child.stderr.write(`${privateStderr}\n`);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(child.stderr.readableLength, 0);
  child.exit(1);

  await assert.rejects(completion, (error) => {
    const rendererMessage = deviceLinkFailureMessage(error);
    assert.equal(rendererMessage, "This desktop could not be linked. Start a new link to try again.");
    assert.doesNotMatch(rendererMessage, new RegExp(privateStderr));
    return true;
  });
});

test("device link drains final stdout data after exit before settling on close", async () => {
  const child = new FakeLinkChild();
  const link = new DeviceLinkSupervisor({
    spawnProcess: () => child,
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeIdentityEnvelope: async () => {},
  });
  const readyPromise = link.begin();
  const completion = link.completion;
  child.stdout.write(
    `${JSON.stringify({
      event: "pairing_ready",
      pairing_session_id: "pairing-public-test",
      target_device_id: "electron-test-device",
    })}\n`
  );
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify(privateDeviceLinkResult("a"))}\n`);
  await new Promise((resolve) => setImmediate(resolve));

  let settled = false;
  completion.finally(() => {
    settled = true;
  });
  child.exit(0, { close: false });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settled, false);

  child.stdout.write('{"event":"linked"}\n');
  child.close();
  await completion;
  assert.equal(settled, true);
});

test("each successful daemon start and restart automatically starts runtime exactly once", async () => {
  const calls = [];
  const state = { status: "runtime started" };
  let finishStart;
  const startGate = new Promise((resolve) => {
    finishStart = resolve;
  });
  const startup = startDaemonRuntime({
    startDaemonProcess: async () => {
      calls.push("start");
      await startGate;
      return true;
    },
    dispatchDaemonAction: async (action) => {
      calls.push(action);
      return state;
    },
  });

  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, ["start"]);
  finishStart();
  assert.equal(await startup, state);

  assert.equal(
    await startDaemonRuntime({
      startDaemonProcess: async () => {
        calls.push("restart");
        return true;
      },
      dispatchDaemonAction: async (action) => {
        calls.push(action);
        return state;
      },
    }),
    state
  );
  assert.equal(
    await startDaemonRuntime({
      startDaemonProcess: async () => {
        calls.push("already-running");
        return false;
      },
      dispatchDaemonAction: async (action) => {
        calls.push(action);
        return state;
      },
    }),
    null
  );
  assert.deepEqual(calls, [
    "start",
    { StartRuntime: null },
    "restart",
    { StartRuntime: null },
    "already-running",
  ]);
});

test("a delayed daemon action response is rejected after daemon restart", async () => {
  const firstConnection = { url: "http://127.0.0.1:44001", authToken: "first" };
  const restartedConnection = { url: "http://127.0.0.1:44002", authToken: "second" };
  let resolveAction;
  const delayedAction = new Promise((resolve) => {
    resolveAction = resolve;
  });
  let currentGeneration = 1;
  let currentConnection = firstConnection;
  const guarded = delayedAction.then((state) => {
    if (!daemonRequestVersionMatches(1, firstConnection, currentGeneration, currentConnection)) {
      throw new Error("Finite Chat request was interrupted by a local service restart");
    }
    return state;
  });

  currentGeneration = 2;
  currentConnection = restartedConnection;
  resolveAction({ rev: 99 });
  await assert.rejects(guarded, /interrupted by a local service restart/);
  assert.equal(daemonRequestVersionMatches(2, restartedConnection, 2, restartedConnection), true);
});

test("restart discards an uncommitted identity envelope after a crash before atomic rename", async (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const store = testDeviceIdentityStore(root);
  const child = new FakeLinkChild();
  let confirmation = "";
  let finishStorage;
  let reportStored;
  const storageGate = new Promise((resolve) => {
    finishStorage = resolve;
  });
  const provisionalStored = new Promise((resolve) => {
    reportStored = resolve;
  });
  child.stdio[4].on("data", (chunk) => {
    confirmation += chunk.toString();
  });
  const link = new DeviceLinkSupervisor({
    spawnProcess: () => child,
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeIdentityEnvelope: async (accountSecret, pendingEnrollment) => {
      store.writeProvisional(identityEnvelope("b", {
        account_secret: accountSecret,
        pending_enrollment: pendingEnrollment,
      }));
      reportStored();
      await storageGate;
      store.promoteProvisional();
    },
  });
  const readyPromise = link.begin();
  const completion = link.completion;
  emitDeviceLinkReady(child);
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify(privateDeviceLinkResult("b"))}\n`);
  await provisionalStored;

  assert.equal(store.read(), null);
  assert.equal(fs.existsSync(store.provisionalPath), true);
  assert.equal(confirmation, "");

  const restartedStore = testDeviceIdentityStore(root);
  restartedStore.discardProvisional();
  assert.equal(restartedStore.read(), null);
  assert.equal(fs.existsSync(restartedStore.provisionalPath), false);

  child.exit(1);
  finishStorage();
  await assert.rejects(completion, /could not securely store/);
  assert.equal(confirmation, "");
});

test("committed identity resumes after a crash between atomic rename and fd4 confirmation", async (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const store = testDeviceIdentityStore(root);
  const child = new FakeLinkChild();
  let confirmation = "";
  let finishStorage;
  let reportCommitted;
  const storageGate = new Promise((resolve) => {
    finishStorage = resolve;
  });
  const committed = new Promise((resolve) => {
    reportCommitted = resolve;
  });
  child.stdio[4].on("data", (chunk) => {
    confirmation += chunk.toString();
  });
  const link = new DeviceLinkSupervisor({
    spawnProcess: () => child,
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeIdentityEnvelope: async (accountSecret, pendingEnrollment) => {
      store.writeProvisional(identityEnvelope("b", {
        account_secret: accountSecret,
        pending_enrollment: pendingEnrollment,
      }));
      store.promoteProvisional();
      reportCommitted();
      await storageGate;
    },
  });
  const readyPromise = link.begin();
  const durable = link.durable;
  const completion = link.completion;
  emitDeviceLinkReady(child);
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify(privateDeviceLinkResult("b"))}\n`);
  await committed;

  assert.deepEqual(store.read(), identityEnvelope("b"));
  assert.equal(confirmation, "");
  child.exit(1);
  finishStorage();
  assert.deepEqual(await durable, identityEnvelope("b").pending_enrollment);
  await assert.rejects(completion, /stopped before completion/);

  const restartedStore = testDeviceIdentityStore(root);
  restartedStore.discardProvisional();
  assert.deepEqual(restartedStore.read(), identityEnvelope("b"));
});

test("committed identity envelope survives a crash after durable storage but before NIP Complete", async (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const store = testDeviceIdentityStore(root);
  const child = new FakeLinkChild();
  let confirmation = "";
  child.stdio[4].on("data", (chunk) => {
    confirmation += chunk.toString();
  });
  const link = new DeviceLinkSupervisor({
    spawnProcess: () => child,
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeIdentityEnvelope: async (accountSecret, pendingEnrollment) => {
      store.writeProvisional(identityEnvelope("c", {
        account_secret: accountSecret,
        pending_enrollment: pendingEnrollment,
      }));
      store.promoteProvisional();
    },
  });
  const readyPromise = link.begin();
  const durable = link.durable;
  const completion = link.completion;
  emitDeviceLinkReady(child);
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify(privateDeviceLinkResult("c"))}\n`);
  const enrollment = await durable;

  assert.equal(confirmation, "stored\n");
  assert.deepEqual(store.read(), identityEnvelope("c"));
  assert.equal(fs.existsSync(store.provisionalPath), false);
  assert.equal(enrollment.enrollment_capability_hex, "ab".repeat(32));

  child.exit(1);
  await assert.rejects(completion, /stopped before completion/);
  const restartedStore = testDeviceIdentityStore(root);
  restartedStore.discardProvisional();
  assert.deepEqual(restartedStore.read(), identityEnvelope("c"));
  assert.equal(fs.existsSync(restartedStore.provisionalPath), false);
});

test("complete identity envelope is promoted before Rust receives its stored confirmation", async (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const store = testDeviceIdentityStore(root);
  const child = new FakeLinkChild();
  const link = new DeviceLinkSupervisor({
    spawnProcess: () => child,
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeIdentityEnvelope: async (accountSecret, pendingEnrollment) => {
      store.writeProvisional(identityEnvelope("d", {
        account_secret: accountSecret,
        pending_enrollment: pendingEnrollment,
      }));
      store.promoteProvisional();
    },
  });
  const readyPromise = link.begin();
  const durable = link.durable;
  const completion = link.completion;
  emitDeviceLinkReady(child);
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify(privateDeviceLinkResult("d"))}\n`);
  await durable;
  assert.deepEqual(store.read(), identityEnvelope("d"));
  assert.equal(fs.existsSync(store.provisionalPath), false);

  child.stdout.write('{"event":"linked"}\n');
  child.exit(0);
  await completion;
  assert.deepEqual(store.read(), identityEnvelope("d"));
  assert.equal(fs.existsSync(store.provisionalPath), false);
});

test("device link never confirms when secure storage fails", async () => {
  const child = new FakeLinkChild();
  let confirmation = "";
  child.stdio[4].on("data", (chunk) => {
    confirmation += chunk.toString();
  });
  const link = new DeviceLinkSupervisor({
    spawnProcess: () => child,
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeIdentityEnvelope: async () => {
      throw new Error("storage unavailable");
    },
  });
  const readyPromise = link.begin();
  const completion = link.completion;
  child.stdout.write(
    `${JSON.stringify({
      event: "pairing_ready",
      pairing_session_id: "pairing-public-test",
      target_device_id: "electron-test-device",
    })}\n`
  );
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify(privateDeviceLinkResult("e"))}\n`);
  await assert.rejects(completion, /securely store/);
  assert.equal(confirmation, "");
});

test("cancelling during secure storage never confirms and waits for the write", async () => {
  const child = new FakeLinkChild();
  let confirmation = "";
  let finishStorage;
  const storageGate = new Promise((resolve) => {
    finishStorage = resolve;
  });
  child.stdio[4].on("data", (chunk) => {
    confirmation += chunk.toString();
  });
  const link = new DeviceLinkSupervisor({
    spawnProcess: () => child,
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeIdentityEnvelope: async () => storageGate,
  });
  const readyPromise = link.begin();
  child.stdout.write(
    `${JSON.stringify({
      event: "pairing_ready",
      pairing_session_id: "pairing-public-test",
      target_device_id: "electron-test-device",
    })}\n`
  );
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify(privateDeviceLinkResult("f"))}\n`);
  await new Promise((resolve) => setImmediate(resolve));
  const cancellation = link.cancel();
  finishStorage();
  await cancellation;
  assert.equal(link.identityStored, true);
  assert.equal(confirmation, "");
});

test("daemon generation always precedes its buffered or live state", () => {
  const live = [];
  const relay = new DaemonUpdateRelay((channel, payload) => live.push([channel, payload]));
  relay.beginGeneration();
  relay.update({ rev: 9 });
  relay.beginGeneration();
  relay.update({ rev: 1 });
  assert.deepEqual(live, [
    ["finitechat:daemon-generation", { generation: 1 }],
    ["finitechat:daemon-update", { rev: 9 }],
    ["finitechat:daemon-generation", { generation: 2 }],
    ["finitechat:daemon-update", { rev: 1 }],
  ]);

  const replay = [];
  relay.replay((channel, payload) => replay.push([channel, payload]));
  assert.deepEqual(replay, [
    ["finitechat:daemon-generation", { generation: 2 }],
    ["finitechat:daemon-update", { rev: 1 }],
  ]);
});

test("daemon relay suppresses duplicate revisions only within one generation", () => {
  const live = [];
  const relay = new DaemonUpdateRelay((channel, payload) => live.push([channel, payload]));

  relay.beginGeneration();
  assert.equal(relay.update({ rev: 7, status: "first" }), true);
  assert.equal(relay.update({ rev: 7, status: "duplicate heartbeat" }), false);
  assert.equal(relay.update({ rev: 8, status: "changed" }), true);
  relay.beginGeneration();
  assert.equal(relay.update({ rev: 7, status: "new daemon baseline" }), true);

  assert.deepEqual(live, [
    ["finitechat:daemon-generation", { generation: 1 }],
    ["finitechat:daemon-update", { rev: 7, status: "first" }],
    ["finitechat:daemon-update", { rev: 8, status: "changed" }],
    ["finitechat:daemon-generation", { generation: 2 }],
    ["finitechat:daemon-update", { rev: 7, status: "new daemon baseline" }],
  ]);
});
