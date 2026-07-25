const fs = require("node:fs");
const path = require("node:path");

const updateFeedUrl =
  "https://github.com/finitecomputer/finite-mono/releases/download/finitechat-latest/finitechat-electron-macos-aarch64-releases.json";
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
    feedUrl = updateFeedUrl,
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
    this.feedUrl = feedUrl;
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

    try {
      this.autoUpdater.setFeedURL({
        url: this.feedUrl,
        serverType: "json",
        headers: { "Cache-Control": "no-cache" },
      });
    } catch (error) {
      this.logger.error(
        `[finitechat-electron] failed to configure app updates: ${boundedErrorMessage(error)}`
      );
      return false;
    }

    this.started = true;
    this.addListener("checking-for-update", () => {
      this.logger.info("[finitechat-electron] checking for an app update");
    });
    this.addListener("update-available", () => {
      this.logger.info("[finitechat-electron] app update available; downloading");
    });
    this.addListener("update-not-available", () => {
      this.checkInFlight = false;
      this.logger.info("[finitechat-electron] app is up to date");
    });
    this.addListener("update-downloaded", (_event, _releaseNotes, releaseName) => {
      this.checkInFlight = false;
      this.updateDownloaded = true;
      const suffix = releaseName ? ` (${String(releaseName).slice(0, 200)})` : "";
      this.logger.info(
        `[finitechat-electron] app update downloaded${suffix}; it will be applied when the app exits`
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
  updateFeedUrl,
};
