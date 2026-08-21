const fs = require("node:fs");
const path = require("node:path");

// The update feed is configured declaratively in `app-update.yml` (written
// into Contents/Resources by scripts/package-macos-alpha.mjs) and consumed by
// electron-updater; this module only schedules checks and forwards lifecycle
// events, mirroring the semantics of the previous custom updater.
const initialUpdateCheckDelayMs = 10 * 1000;
const updateCheckIntervalMs = 6 * 60 * 60 * 1000;

function packagedAutoUpdateEnabled(packagePath = path.resolve(__dirname, "..", "package.json")) {
  try {
    const metadata = JSON.parse(fs.readFileSync(packagePath, "utf8"));
    return metadata.finitechatAutoUpdate === true;
  } catch {
    return false;
  }
}

function boundedErrorMessage(error) {
  const message = error instanceof Error ? error.message : String(error);
  return message.replaceAll(/[\r\n]+/gu, " ").slice(0, 1000);
}

class AppUpdater {
  constructor({
    app,
    autoUpdater,
    enabled,
    platform = process.platform,
    arch = process.arch,
    initialDelayMs = initialUpdateCheckDelayMs,
    checkIntervalMs = updateCheckIntervalMs,
    logger = console,
    setTimeoutFn = setTimeout,
    setIntervalFn = setInterval,
    clearTimeoutFn = clearTimeout,
    clearIntervalFn = clearInterval,
  }) {
    this.app = app;
    this.autoUpdater = autoUpdater;
    this.enabled = enabled;
    this.platform = platform;
    this.arch = arch;
    this.initialDelayMs = initialDelayMs;
    this.checkIntervalMs = checkIntervalMs;
    this.logger = logger;
    this.setTimeoutFn = setTimeoutFn;
    this.setIntervalFn = setIntervalFn;
    this.clearTimeoutFn = clearTimeoutFn;
    this.clearIntervalFn = clearIntervalFn;
    this.started = false;
    this.checkInFlight = false;
    this.updateDownloaded = false;
    this.initialTimer = null;
    this.intervalTimer = null;
    this.listeners = [];
  }

  supported() {
    return (
      this.enabled === true
      && this.app.isPackaged === true
      && this.platform === "darwin"
      && this.arch === "arm64"
    );
  }

  start() {
    if (this.started || !this.supported()) {
      return false;
    }

    this.started = true;
    this.addListener("checking-for-update", () => {
      this.logger.info("[finitechat-electron] checking for an app update");
    });
    this.addListener("update-available", (updateInfo) => {
      this.logger.info(
        `[finitechat-electron] app update ${boundedVersion(updateInfo)} available; downloading`
      );
    });
    this.addListener("update-not-available", () => {
      this.checkInFlight = false;
      this.logger.info("[finitechat-electron] app is up to date");
    });
    this.addListener("update-downloaded", (event) => {
      this.checkInFlight = false;
      this.updateDownloaded = true;
      this.logger.info(
        `[finitechat-electron] app update ${boundedVersion(event)} downloaded; it will be applied when the app exits`
      );
    });
    this.addListener("error", (error) => {
      this.checkInFlight = false;
      this.logger.error(
        `[finitechat-electron] app update failed: ${boundedErrorMessage(error)}`
      );
    });

    const runCheck = () => {
      void this.checkForUpdates();
    };
    this.initialTimer = this.setTimeoutFn(runCheck, this.initialDelayMs);
    this.intervalTimer = this.setIntervalFn(runCheck, this.checkIntervalMs);
    this.initialTimer?.unref?.();
    this.intervalTimer?.unref?.();
    return true;
  }

  async checkForUpdates() {
    if (!this.started || this.checkInFlight || this.updateDownloaded) {
      return false;
    }

    this.checkInFlight = true;
    try {
      await this.autoUpdater.checkForUpdates();
      return true;
    } catch (error) {
      this.checkInFlight = false;
      this.logger.error(
        `[finitechat-electron] app update check failed: ${boundedErrorMessage(error)}`
      );
      return false;
    }
  }

  stop() {
    if (this.initialTimer) {
      this.clearTimeoutFn(this.initialTimer);
      this.initialTimer = null;
    }
    if (this.intervalTimer) {
      this.clearIntervalFn(this.intervalTimer);
      this.intervalTimer = null;
    }
    for (const [eventName, listener] of this.listeners) {
      this.autoUpdater.removeListener(eventName, listener);
    }
    this.listeners = [];
    this.started = false;
    this.checkInFlight = false;
  }

  addListener(eventName, listener) {
    this.autoUpdater.on(eventName, listener);
    this.listeners.push([eventName, listener]);
  }
}

function boundedVersion(updateInfo) {
  const version = updateInfo?.version === undefined ? "" : String(updateInfo.version);
  return version ? ` ${version.slice(0, 200)}` : "";
}

function createAppUpdater({ app, autoUpdater, ...options }) {
  return new AppUpdater({
    app,
    autoUpdater,
    enabled: packagedAutoUpdateEnabled(),
    ...options,
  });
}

module.exports = {
  AppUpdater,
  boundedErrorMessage,
  createAppUpdater,
  packagedAutoUpdateEnabled,
};
