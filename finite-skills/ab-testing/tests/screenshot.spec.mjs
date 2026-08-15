import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { expect, test } from "@playwright/test";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const latestDir = path.join(harnessRoot, "runs/latest");
const artifactsDir = path.join(latestDir, "artifacts");
const reviewDir = path.join(latestDir, "review");
const screenshotDir = path.join(reviewDir, "screenshots");
const records = [];

const viewports = [
  { name: "desktop", width: 1440, height: 1000 },
  { name: "mobile", width: 390, height: 844 },
];

mkdirSync(screenshotDir, { recursive: true });

test.describe.configure({ mode: "serial" });

for (const artifact of collectArtifacts()) {
  for (const viewport of viewports) {
    test(`${artifact.caseId} / ${artifact.variant} / ${viewport.name}`, async ({ page }) => {
      const consoleErrors = [];
      page.on("console", (message) => {
        if (message.type() === "error") {
          consoleErrors.push(message.text());
        }
      });
      page.on("pageerror", (error) => {
        consoleErrors.push(error.message);
      });

      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      await page.goto(pathToFileURL(artifact.htmlPath).href);
      await page.waitForLoadState("load");
      await page.waitForTimeout(100);
      const renderState = await page.evaluate(() => {
        const body = document.body;
        const rect = body?.getBoundingClientRect();
        return {
          height: rect?.height ?? 0,
          textLength: body?.innerText?.trim().length ?? 0,
          width: rect?.width ?? 0,
        };
      });
      expect(renderState.textLength, "rendered page should contain visible body text").toBeGreaterThan(20);
      expect(renderState.width * renderState.height, "rendered page should occupy visible page area").toBeGreaterThan(10000);
      const filename = `${artifact.caseId}--${artifact.variant}--${viewport.name}.png`;
      const screenshotPath = path.join(screenshotDir, filename);
      await page.screenshot({ fullPage: true, path: screenshotPath });
      records.push({
        caseId: artifact.caseId,
        consoleErrors,
        height: viewport.height,
        relativePath: `screenshots/${filename}`,
        variant: artifact.variant,
        viewport: viewport.name,
        width: viewport.width,
      });
    });
  }
}

test.afterAll(() => {
  writeFileSync(path.join(reviewDir, "screenshots.json"), JSON.stringify(records, null, 2), "utf8");
});

function collectArtifacts() {
  if (!existsSync(artifactsDir)) {
    return [];
  }
  const artifacts = [];
  for (const caseId of sortedDirs(artifactsDir)) {
    for (const variant of sortedDirs(path.join(artifactsDir, caseId))) {
      const dir = path.join(artifactsDir, caseId, variant);
      const metadata = JSON.parse(readFileSync(path.join(dir, "metadata.json"), "utf8"));
      artifacts.push({
        caseId,
        htmlPath: path.join(dir, "index.html"),
        metadata,
        variant,
      });
    }
  }
  return artifacts;
}

function sortedDirs(dir) {
  return readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();
}
