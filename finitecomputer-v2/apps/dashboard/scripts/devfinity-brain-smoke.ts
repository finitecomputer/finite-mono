import assert from "node:assert/strict";

import { chromium, type Browser, type FrameLocator, type Page } from "playwright";

import { chromiumLaunchOptions } from "./playwright-browser";

async function main() {
  const action = process.argv[2];
  const dashboardUrl = requiredEnv("FC_DASHBOARD_URL").replace(/\/$/u, "");
  const machineId = requiredEnv("DEVFINITY_BRAIN_MACHINE_ID");
  const agentNpub = requiredEnv("DEVFINITY_BRAIN_AGENT_NPUB");
  const expectedText = process.env.DEVFINITY_BRAIN_EXPECTED_TEXT?.trim() || "";

  if (action !== "pair" && action !== "assert-note") {
    throw new Error("usage: devfinity-brain-smoke.ts pair|assert-note");
  }
  if (!agentNpub.startsWith("npub1")) {
    throw new Error("DEVFINITY_BRAIN_AGENT_NPUB must be an npub");
  }
  if (action === "assert-note" && !expectedText) {
    throw new Error("DEVFINITY_BRAIN_EXPECTED_TEXT is required for assert-note");
  }

  let browser: Browser | null = null;
  const diagnostics: string[] = [];

  try {
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const context = await browser.newContext();
    const page = await context.newPage();
    page.on("console", (message) => {
      if (message.type() === "error") diagnostics.push(`console: ${message.text()}`);
    });
    page.on("pageerror", (error) => diagnostics.push(`pageerror: ${error.message}`));

    await page.goto(`${dashboardUrl}/machines/${encodeURIComponent(machineId)}/brain`, {
      waitUntil: "domcontentloaded",
      timeout: 60_000,
    });
    const brain = page.frameLocator('iframe[title$=" Brain"]');
    await waitForUnlockedBrain(brain, page);

    if (action === "pair") {
      await pairAgent(brain, agentNpub);
      console.log("brain owner pairing ok");
    } else {
      await assertOwnerSeesNote(brain, expectedText);
      console.log("brain owner readback ok");
    }

    await context.close();
  } catch (error) {
    const detail = diagnostics.length ? `\n${diagnostics.join("\n")}` : "";
    throw new Error(`${error instanceof Error ? error.message : String(error)}${detail}`);
  } finally {
    await browser?.close().catch(() => {});
  }
}

void main();

async function waitForUnlockedBrain(brain: FrameLocator, page: Page) {
  const status = brain.locator("#sessionAccountStatus");
  await status.waitFor({ state: "visible", timeout: 90_000 }).catch(async (error) => {
    throw new Error(
      `Brain Product Client did not render: ${String(error)}\n${await page.locator("body").innerText()}`
    );
  });
  await assertEventually(
    async () => (await status.textContent())?.trim() === "Session unlocked",
    90_000,
    async () => `Brain did not unlock; current status: ${(await status.textContent())?.trim()}`
  );
}

async function pairAgent(brain: FrameLocator, expectedAgentNpub: string) {
  await brain.locator("#sessionSettingsButton").click();
  await brain.locator("#settingsNavAccess").click();
  const section = brain.locator("#agentWorkspacePairingSection");
  await section.waitFor({ state: "visible", timeout: 30_000 });

  const pairingCount = brain.locator("#agentWorkspacePairingCount");
  const input = brain.locator("#agentWorkspaceNpubInput");
  const count = Number((await pairingCount.textContent())?.trim() || "0");
  if (count === 0) {
    assert.equal(
      await input.inputValue(),
      expectedAgentNpub,
      "the dashboard navigation hint must prefill the selected Agent Principal"
    );
    await brain.locator("#pairAgentWorkspaceButton").click();
    await brain
      .locator("#accessResultPanel")
      .getByText("Agent paired", { exact: true })
      .waitFor({ state: "visible", timeout: 60_000 });
  }

  await assertEventually(
    async () => Number((await pairingCount.textContent())?.trim() || "0") >= 1,
    30_000,
    async () => `Agent pairing count never advanced: ${(await pairingCount.textContent())?.trim()}`
  );
}

async function assertOwnerSeesNote(brain: FrameLocator, expectedText: string) {
  const refresh = brain.locator("#refreshReaderButton");
  await assertEventually(
    async () => !(await refresh.isDisabled()),
    30_000,
    async () => "Brain refresh did not become available"
  );
  await refresh.click();
  await brain
    .locator("body")
    .getByText(expectedText, { exact: false })
    .first()
    .waitFor({ state: "visible", timeout: 60_000 });
}

async function assertEventually(
  predicate: () => Promise<boolean>,
  timeoutMs: number,
  failure: () => Promise<string>
) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(await failure());
}

function requiredEnv(name: string) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required`);
  return value;
}
