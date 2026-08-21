const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  AppUpdater,
  boundedErrorMessage,
  packagedAutoUpdateEnabled,
} = require("./app-updater.cjs");

class FakeAutoUpdater extends EventEmitter {
  constructor() {
    super();
    this.checkCount = 0;
    this.checkResult = Promise.resolve();
  }

  checkForUpdates() {
    this.checkCount += 1;
    return this.checkResult;
  }
}

function updaterHarness(overrides = {}) {
  const autoUpdater = overrides.autoUpdater || new FakeAutoUpdater();
  const timeouts = [];
  const intervals = [];
  const logs = [];
  const errors = [];
  const updater = new AppUpdater({
    app: { isPackaged: true },
    autoUpdater,
    enabled: true,
    platform: "darwin",
    arch: "arm64",
    logger: {
      info(message) {
        logs.push(message);
      },
      error(message) {
        errors.push(message);
      },
    },
    setTimeoutFn(callback, milliseconds) {
      const timer = { callback, milliseconds, unrefCalled: false };
      timer.unref = () => {
        timer.unrefCalled = true;
      };
      timeouts.push(timer);
      return timer;
    },
    setIntervalFn(callback, milliseconds) {
      const timer = { callback, milliseconds, unrefCalled: false };
      timer.unref = () => {
        timer.unrefCalled = true;
      };
      intervals.push(timer);
      return timer;
    },
    clearTimeoutFn() {},
    clearIntervalFn() {},
    ...overrides,
  });
  return { updater, autoUpdater, timeouts, intervals, logs, errors };
}

test("release updater stays disabled outside a marked arm64 macOS package", () => {
  for (const config of [
    { app: { isPackaged: false } },
    { enabled: false },
    { platform: "linux" },
    { arch: "x64" },
  ]) {
    const { updater, timeouts, intervals } = updaterHarness(config);
    assert.equal(updater.start(), false);
    assert.equal(updater.started, false);
    assert.equal(timeouts.length, 0);
    assert.equal(intervals.length, 0);
  }
});

test("release updater schedules checks without checking synchronously", () => {
  const { updater, autoUpdater, timeouts, intervals } = updaterHarness();

  assert.equal(updater.start(), true);
  assert.equal(autoUpdater.checkCount, 0);
  assert.equal(timeouts.length, 1);
  assert.equal(intervals.length, 1);
  assert.equal(timeouts[0].milliseconds, 10 * 1000);
  assert.equal(intervals[0].milliseconds, 6 * 60 * 60 * 1000);
  assert.equal(timeouts[0].unrefCalled, true);
  assert.equal(intervals[0].unrefCalled, true);
  assert.equal(updater.start(), false);
  assert.equal(timeouts.length, 1);
  assert.equal(intervals.length, 1);
});

test("scheduled checks coalesce until the updater reaches a terminal event", async () => {
  const { updater, autoUpdater, timeouts, intervals } = updaterHarness();
  updater.start();

  timeouts[0].callback();
  await Promise.resolve();
  assert.equal(autoUpdater.checkCount, 1);

  intervals[0].callback();
  await Promise.resolve();
  assert.equal(autoUpdater.checkCount, 1);

  autoUpdater.emit("update-not-available");
  intervals[0].callback();
  await Promise.resolve();
  assert.equal(autoUpdater.checkCount, 2);
});

test("a downloaded update prevents redundant checks until relaunch", async () => {
  const { updater, autoUpdater, timeouts, intervals, logs } = updaterHarness();
  updater.start();

  timeouts[0].callback();
  await Promise.resolve();
  autoUpdater.emit("update-downloaded", { version: "0.1.9", downloadedFile: "/tmp/x.zip" });
  intervals[0].callback();
  await Promise.resolve();

  assert.equal(autoUpdater.checkCount, 1);
  assert.match(logs.at(-1), /0\.1\.9 .*will be applied when the app exits/u);
});

test("rejected update checks clear the in-flight guard and stay bounded", async () => {
  const autoUpdater = new FakeAutoUpdater();
  autoUpdater.checkResult = Promise.reject(new Error(`network\n${"x".repeat(2000)}`));
  const { updater, errors } = updaterHarness({ autoUpdater });
  updater.start();

  assert.equal(await updater.checkForUpdates(), false);
  assert.equal(updater.checkInFlight, false);
  assert.equal(errors.length, 1);
  assert.equal(errors[0].includes("\n"), false);
  assert.ok(errors[0].length < 1100);
});

test("only packaged metadata explicitly opts into production updates", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "finitechat-updater-"));
  const packagePath = path.join(directory, "package.json");
  try {
    fs.writeFileSync(packagePath, JSON.stringify({ finitechatAutoUpdate: true }));
    assert.equal(packagedAutoUpdateEnabled(packagePath), true);
    fs.writeFileSync(packagePath, JSON.stringify({ finitechatAutoUpdate: false }));
    assert.equal(packagedAutoUpdateEnabled(packagePath), false);
    fs.writeFileSync(packagePath, "{");
    assert.equal(packagedAutoUpdateEnabled(packagePath), false);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

test("error messages remove line breaks and enforce a fixed bound", () => {
  const message = boundedErrorMessage(new Error(`first\nsecond${"z".repeat(2000)}`));
  assert.equal(message.includes("\n"), false);
  assert.equal(message.length, 1000);
});
