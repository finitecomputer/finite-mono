import assert from "node:assert/strict";

import {
  chromium,
  type Browser,
  type FrameLocator,
  type Page,
} from "playwright";

import { chromiumLaunchOptions } from "./playwright-browser";

async function main() {
  const action = process.argv[2];
  const dashboardUrl = requiredEnv("FC_DASHBOARD_URL").replace(/\/$/u, "");
  const machineId = requiredEnv("DEVFINITY_BRAIN_MACHINE_ID");
  const agentEmail = requiredEnv("DEVFINITY_BRAIN_AGENT_EMAIL").toLowerCase();
  const expectedText = process.env.DEVFINITY_BRAIN_EXPECTED_TEXT?.trim() || "";

  if (action !== "bootstrap" && action !== "assert-note") {
    throw new Error("usage: devfinity-brain-smoke.ts bootstrap|assert-note");
  }
  if (!agentEmail.includes("@")) {
    throw new Error("DEVFINITY_BRAIN_AGENT_EMAIL must be an email");
  }
  if (action === "assert-note" && !expectedText) {
    throw new Error(
      "DEVFINITY_BRAIN_EXPECTED_TEXT is required for assert-note",
    );
  }

  let browser: Browser | null = null;
  const diagnostics: string[] = [];

  try {
    browser = await chromium.launch({
      headless: true,
      ...chromiumLaunchOptions(),
    });
    const context = await browser.newContext();
    const page = await context.newPage();
    let personalAgentConfirmation = "";
    page.on("dialog", async (dialog) => {
      if (action !== "bootstrap" || dialog.type() !== "confirm") {
        await dialog.dismiss();
        return;
      }
      personalAgentConfirmation = dialog.message();
      await dialog.accept();
    });
    page.on("console", (message) => {
      if (message.type() === "error") {
        const location = message.location();
        diagnostics.push(
          `console (${location.url || "unknown"}:${location.lineNumber}): ${message.text()}`,
        );
      }
    });
    page.on("pageerror", (error) =>
      diagnostics.push(`pageerror: ${error.message}`),
    );

    await page.goto(
      `${dashboardUrl}/machines/${encodeURIComponent(machineId)}/brain`,
      {
        waitUntil: "domcontentloaded",
        timeout: 60_000,
      },
    );
    const brain = page.frameLocator('iframe[title$=" Brain"]');
    await waitForUnlockedBrain(brain, page);

    if (action === "bootstrap") {
      assert.ok(
        personalAgentConfirmation.toLowerCase().includes(agentEmail),
        `Personal Agent confirmation did not show ${agentEmail}: ${personalAgentConfirmation}`,
      );
      await assertPersonalAgent(brain, agentEmail);
      console.log("brain user-first Personal Agent bootstrap ok");
    } else {
      await assertOwnerSeesNote(brain, expectedText);
      console.log("brain owner readback ok");
    }

    await context.close();
  } catch (error) {
    const detail = diagnostics.length ? `\n${diagnostics.join("\n")}` : "";
    throw new Error(
      `${error instanceof Error ? error.message : String(error)}${detail}`,
    );
  } finally {
    await browser?.close().catch(() => {});
  }
}

void main();

async function waitForUnlockedBrain(brain: FrameLocator, page: Page) {
  const timeoutMs = Number(process.env.DEVFINITY_BRAIN_TIMEOUT_MS || 90_000);
  const status = brain.locator("#sessionAccountStatus");
  await status
    .waitFor({ state: "visible", timeout: timeoutMs })
    .catch(async (error) => {
      throw new Error(
        `Brain Product Client did not render: ${String(error)}\n${await page.locator("body").innerText()}`,
      );
    });
  await assertEventually(
    async () => (await status.textContent())?.trim() === "Session unlocked",
    timeoutMs,
    async () =>
      `Brain did not unlock; current status: ${(await status.textContent())?.trim()}`,
  );
}

async function assertPersonalAgent(
  brain: FrameLocator,
  expectedAgentEmail: string,
) {
  await brain.locator("#sessionSettingsButton").click();
  await brain.locator("#settingsNavAccess").click();
  const section = brain.locator("#personalAgentSection");
  await section.waitFor({ state: "visible", timeout: 30_000 });
  const current = brain.locator("#personalAgentCurrent");
  await assertEventually(
    async () =>
      (await current.textContent())
        ?.toLowerCase()
        .includes(expectedAgentEmail) === true,
    30_000,
    async () =>
      `Personal Agent did not resolve to ${expectedAgentEmail}: ${(await current.textContent())?.trim()}`,
  );
  assert.equal(
    await brain.locator("#personalAgentEmailInput").getAttribute("placeholder"),
    "agent@finite.computer",
  );
}

async function assertOwnerSeesNote(brain: FrameLocator, expectedText: string) {
  const timeoutMs = Number(process.env.DEVFINITY_BRAIN_TIMEOUT_MS || 60_000);
  const refresh = brain.locator("#refreshReaderButton");
  await assertEventually(
    async () => !(await refresh.isDisabled()),
    30_000,
    async () => "Brain refresh did not become available",
  );
  await refresh.click();
  await brain
    .locator("body")
    .getByText(expectedText, { exact: false })
    .first()
    .waitFor({ state: "visible", timeout: timeoutMs })
    .catch(async (error) => {
      throw new Error(
        `${String(error)}\nBrain content: ${(await brain.locator("body").innerText()).slice(0, 4000)}`,
      );
    });
}

async function assertEventually(
  predicate: () => Promise<boolean>,
  timeoutMs: number,
  failure: () => Promise<string>,
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
