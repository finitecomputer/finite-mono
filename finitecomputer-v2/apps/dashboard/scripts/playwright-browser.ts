import { existsSync, readdirSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join } from "node:path";

import { chromium } from "playwright";

export function chromiumLaunchOptions() {
  const configured = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH?.trim();
  if (configured && existsSync(configured)) return { executablePath: configured };
  for (const executablePath of [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
  ]) {
    if (existsSync(executablePath)) return { executablePath };
  }
  // Prefer Playwright's known-good browser before PATH shims. Package-manager
  // launchers can remain executable after their application bundle is removed,
  // which makes an existence-only PATH check select a broken Chromium wrapper.
  const playwrightChromium = chromium.executablePath();
  if (existsSync(playwrightChromium)) return { executablePath: playwrightChromium };
  for (const cacheRoot of [
    join(homedir(), "Library", "Caches", "ms-playwright"),
    join(homedir(), ".cache", "ms-playwright"),
  ]) {
    if (!existsSync(cacheRoot)) continue;
    const releases = readdirSync(cacheRoot)
      .filter((name) => name.startsWith("chromium_headless_shell-"))
      .sort()
      .reverse();
    for (const release of releases) {
      for (const relative of [
        "chrome-headless-shell-mac-arm64/chrome-headless-shell",
        "chrome-headless-shell-linux64/chrome-headless-shell",
      ]) {
        const executablePath = join(cacheRoot, release, relative);
        if (existsSync(executablePath)) return { executablePath };
      }
    }
  }
  for (const directory of (process.env.PATH || "").split(delimiter)) {
    for (const executable of ["google-chrome", "chromium", "chromium-browser"]) {
      const executablePath = join(directory, executable);
      if (existsSync(executablePath)) return { executablePath };
    }
  }
  return { channel: "chrome" as const };
}
