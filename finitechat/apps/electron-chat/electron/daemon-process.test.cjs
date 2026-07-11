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
  AccountSecretStore,
  DaemonUpdateRelay,
  DaemonSupervisor,
  DeviceLinkSupervisor,
  daemonRequestVersionMatches,
  legacyHostnameDeviceId,
  loadOrCreateDeviceId,
  parseReadyRecord,
  parseDeviceLinkReadyRecord,
  parseDeviceLinkSecretRecord,
  resolveDaemonBinary,
  startDaemonRuntime,
  startupDocument,
  validDeviceLinkApproval,
} = require("./daemon-process.cjs");

function temporaryDirectory() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "finitechat-electron-process-"));
}

function testSafeStorage() {
  return {
    isEncryptionAvailable: () => true,
    getSelectedStorageBackend: () => "keychain",
    encryptString: (value) => Buffer.from(value, "utf8").reverse(),
    decryptString: (value) => Buffer.from(value).reverse().toString("utf8"),
  };
}

function testAccountSecretStore(root) {
  return new AccountSecretStore({
    secretPath: path.join(root, "account-secret.safe"),
    safeStorage: testSafeStorage(),
  });
}

function emitDeviceLinkReady(child) {
  child.stdout.write(
    `${JSON.stringify({
      event: "link_ready",
      link_session_id: "link-public-test",
      target_device_id: "electron-test-device",
      approval_url:
        "https://finite.computer/dashboard/device-link?link_session_id=link-public-test&target_device_id=electron-test-device",
    })}\n`
  );
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

test("renderer attachment boundary has no raw daemon network fallback", () => {
  const daemonSource = fs.readFileSync(path.resolve(__dirname, "../src/daemon.ts"), "utf8");
  const appSource = fs.readFileSync(path.resolve(__dirname, "../src/App.tsx"), "utf8");
  const html = fs.readFileSync(path.resolve(__dirname, "../index.html"), "utf8");
  assert.doesNotMatch(daemonSource, /VITE_FINITECHAT_DAEMON_URL|new EventSource|\bfetch\s*\(/);
  assert.doesNotMatch(html, /127\.0\.0\.1|ws:\/\//);
  assert.match(html, /img-src[^;]*finitechat-media:/);
  assert.match(appSource, /attachmentSendError\(next\)/);
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
      event: "link_ready",
      link_session_id: "link-public-test",
      target_device_id: "electron-test-device",
      approval_url:
        "https://finite.computer/dashboard/device-link?link_session_id=link-public-test&target_device_id=electron-test-device",
    })
  );
  assert.equal(ready.link_session_id, "link-public-test");
  assert.equal(validDeviceLinkApproval(ready, "https://finite.computer"), true);
  assert.equal(validDeviceLinkApproval(ready, "https://operator.example"), false);
  assert.equal(parseDeviceLinkSecretRecord(JSON.stringify({ account_secret: "c".repeat(64) })), "c".repeat(64));
  assert.throws(
    () => parseDeviceLinkReadyRecord('{"event":"link_ready","approval_url":"file:///tmp/secret"}'),
    /invalid status record/
  );
  assert.throws(
    () => parseDeviceLinkSecretRecord(JSON.stringify({ account_secret: "not-secret-material" })),
    /invalid private result/
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
    this.stdio = [null, this.stdout, this.stderr, new PassThrough(), new PassThrough()];
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
  const storedSecrets = [];
  const promotions = [];
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
    dashboardUrl: "https://finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeAccountSecret: async (secret) => storedSecrets.push(secret),
    promoteAccountSecret: async () => promotions.push("promoted"),
  });
  const readyPromise = link.begin();
  const completion = link.completion;
  child.stdout.write(
    `${JSON.stringify({
      event: "link_ready",
      link_session_id: "link-public-test",
      target_device_id: "electron-test-device",
      approval_url:
        "https://finite.computer/dashboard/device-link?link_session_id=link-public-test&target_device_id=electron-test-device",
    })}\n`
  );
  const ready = await readyPromise;
  assert.equal(ready.target_device_id, "electron-test-device");
  assert.deepEqual(spawnOptions.stdio, ["ignore", "pipe", "pipe", "pipe", "pipe"]);
  assert.deepEqual(spawnArgs, [
    "link",
    "--server-url",
    "https://chat.finite.computer",
    "--dashboard-url",
    "https://finite.computer",
    "--device-id",
    "electron-test-device",
    "--result-fd",
    "3",
    "--confirm-fd",
    "4",
  ]);

  child.stdio[3].write(`${JSON.stringify({ account_secret: "d".repeat(64) })}\n`);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(storedSecrets, ["d".repeat(64)]);
  assert.equal(confirmation, "stored\n");
  assert.deepEqual(promotions, []);
  child.stdout.write('{"event":"linked"}\n');
  child.exit(0);
  await completion;
  assert.deepEqual(promotions, ["promoted"]);
});

test("device link drains final stdout data after exit before settling on close", async () => {
  const child = new FakeLinkChild();
  const link = new DeviceLinkSupervisor({
    spawnProcess: () => child,
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    dashboardUrl: "https://finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeAccountSecret: async () => {},
    promoteAccountSecret: async () => {},
  });
  const readyPromise = link.begin();
  const completion = link.completion;
  child.stdout.write(
    `${JSON.stringify({
      event: "link_ready",
      link_session_id: "link-public-test",
      target_device_id: "electron-test-device",
      approval_url:
        "https://finite.computer/dashboard/device-link?link_session_id=link-public-test&target_device_id=electron-test-device",
    })}\n`
  );
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify({ account_secret: "a".repeat(64) })}\n`);
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

test("restart discards a provisional secret after a crash between fd3 and fd4", async (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const store = testAccountSecretStore(root);
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
    dashboardUrl: "https://finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeAccountSecret: async (secret) => {
      store.writeProvisional(secret);
      reportStored();
      await storageGate;
    },
    promoteAccountSecret: async () => store.promoteProvisional(),
  });
  const readyPromise = link.begin();
  const completion = link.completion;
  emitDeviceLinkReady(child);
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify({ account_secret: "b".repeat(64) })}\n`);
  await provisionalStored;

  assert.equal(store.read(), null);
  assert.equal(fs.existsSync(store.provisionalPath), true);
  assert.equal(confirmation, "");

  const restartedStore = testAccountSecretStore(root);
  restartedStore.discardProvisional();
  assert.equal(restartedStore.read(), null);
  assert.equal(fs.existsSync(restartedStore.provisionalPath), false);

  child.exit(1);
  finishStorage();
  await assert.rejects(completion, /stopped before completion/);
  assert.equal(confirmation, "");
});

test("restart discards a provisional secret after fd4 but before linked", async (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const store = testAccountSecretStore(root);
  const child = new FakeLinkChild();
  let confirmation = "";
  child.stdio[4].on("data", (chunk) => {
    confirmation += chunk.toString();
  });
  const link = new DeviceLinkSupervisor({
    spawnProcess: () => child,
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    dashboardUrl: "https://finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeAccountSecret: async (secret) => store.writeProvisional(secret),
    promoteAccountSecret: async () => store.promoteProvisional(),
  });
  const readyPromise = link.begin();
  const completion = link.completion;
  emitDeviceLinkReady(child);
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify({ account_secret: "c".repeat(64) })}\n`);
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(confirmation, "stored\n");
  assert.equal(store.read(), null);
  assert.equal(fs.existsSync(store.provisionalPath), true);

  child.exit(1);
  await assert.rejects(completion, /stopped before completion/);
  const restartedStore = testAccountSecretStore(root);
  restartedStore.discardProvisional();
  assert.equal(restartedStore.read(), null);
  assert.equal(fs.existsSync(restartedStore.provisionalPath), false);
});

test("the final linked event promotes the provisional safe-storage file", async (context) => {
  const root = temporaryDirectory();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const store = testAccountSecretStore(root);
  const child = new FakeLinkChild();
  const link = new DeviceLinkSupervisor({
    spawnProcess: () => child,
    binaryPath: "/tmp/finitechatd",
    serverUrl: "https://chat.finite.computer",
    dashboardUrl: "https://finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeAccountSecret: async (secret) => store.writeProvisional(secret),
    promoteAccountSecret: async () => store.promoteProvisional(),
  });
  const readyPromise = link.begin();
  const completion = link.completion;
  emitDeviceLinkReady(child);
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify({ account_secret: "d".repeat(64) })}\n`);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(store.read(), null);

  child.stdout.write('{"event":"linked"}\n');
  child.exit(0);
  await completion;
  assert.equal(store.read(), "d".repeat(64));
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
    dashboardUrl: "https://finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeAccountSecret: async () => {
      throw new Error("storage unavailable");
    },
  });
  const readyPromise = link.begin();
  const completion = link.completion;
  child.stdout.write(
    `${JSON.stringify({
      event: "link_ready",
      link_session_id: "link-public-test",
      target_device_id: "electron-test-device",
      approval_url:
        "https://finite.computer/dashboard/device-link?link_session_id=link-public-test&target_device_id=electron-test-device",
    })}\n`
  );
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify({ account_secret: "e".repeat(64) })}\n`);
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
    dashboardUrl: "https://finite.computer",
    deviceId: "electron-test-device",
    cwd: "/tmp",
    storeAccountSecret: async () => storageGate,
  });
  const readyPromise = link.begin();
  child.stdout.write(
    `${JSON.stringify({
      event: "link_ready",
      link_session_id: "link-public-test",
      target_device_id: "electron-test-device",
      approval_url:
        "https://finite.computer/dashboard/device-link?link_session_id=link-public-test&target_device_id=electron-test-device",
    })}\n`
  );
  await readyPromise;
  child.stdio[3].write(`${JSON.stringify({ account_secret: "f".repeat(64) })}\n`);
  await new Promise((resolve) => setImmediate(resolve));
  const cancellation = link.cancel();
  finishStorage();
  await cancellation;
  assert.equal(link.secretStored, true);
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
