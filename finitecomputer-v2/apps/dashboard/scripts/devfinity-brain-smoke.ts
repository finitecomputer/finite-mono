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
  const targetBrainId = process.env.DEVFINITY_BRAIN_TARGET_ID?.trim() || "";

  const actions = new Set([
    "bootstrap",
    "assert-existing-personal",
    "assert-org-first",
    "assert-note",
    "create-org-agent",
    "create-org-human",
    "create-folder",
    "assert-folder",
  ]);
  if (!actions.has(action)) {
    throw new Error(
      "usage: devfinity-brain-smoke.ts bootstrap|assert-existing-personal|assert-org-first|assert-note|create-org-agent|create-org-human|create-folder|assert-folder",
    );
  }
  if (!agentEmail.includes("@")) {
    throw new Error("DEVFINITY_BRAIN_AGENT_EMAIL must be an email");
  }
  if (["assert-note", "assert-org-first", "create-org-agent", "create-org-human", "create-folder", "assert-folder"].includes(action) && !expectedText) {
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
      if (action === "create-folder" && dialog.type() === "prompt") {
        await dialog.accept(expectedText);
        return;
      }
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

    const directBrainTarget = (targetBrainId || action === "assert-org-first")
      ? `?brainId=${encodeURIComponent(targetBrainId || expectedText)}`
      : "";
    await page.goto(
      `${dashboardUrl}/machines/${encodeURIComponent(machineId)}/brain${directBrainTarget}`,
      {
        waitUntil: "domcontentloaded",
        timeout: 60_000,
      },
    );
    const brainFrame = page.locator('iframe[title$=" Brain"]');
    await brainFrame.waitFor({ state: "visible", timeout: 60_000 });
    assert.match(
      (await brainFrame.getAttribute("sandbox")) || "",
      /(?:^|\s)allow-modals(?:\s|$)/u,
      "Hosted Brain frame must permit its bounded Product Client dialogs",
    );
    const brain = page.frameLocator('iframe[title$=" Brain"]');

    if (action === "bootstrap") {
      await waitForBrainClient(brain, page);
      await createPersonalBrain(brain, agentEmail);
      await waitForUnlockedBrain(brain, page);
      await closeManageBrainsIfOpen(brain);
      if (personalAgentConfirmation) {
        assert.ok(
          personalAgentConfirmation.toLowerCase().includes(agentEmail),
          `Personal Agent confirmation did not show ${agentEmail}: ${personalAgentConfirmation}`,
        );
      }
      await assertPersonalAgent(brain, agentEmail);
      console.log(`BRAIN_ID=${await selectedBrainId(brain)}`);
      console.log("brain user-first Personal Agent bootstrap ok");
    } else if (action === "assert-existing-personal") {
      await waitForUnlockedBrain(brain, page);
      assert.equal(personalAgentConfirmation, "");
      await assertPersonalAgent(brain, agentEmail);
      console.log(`BRAIN_ID=${await selectedBrainId(brain)}`);
      console.log("brain agent-first Personal Agent bootstrap ok");
    } else if (action === "assert-org-first") {
      await waitForUnlockedBrain(brain, page);
      await assertOrgFirstBrain(brain, expectedText);
      console.log(`BRAIN_ID=${await selectedBrainId(brain)}`);
      console.log("brain agent-first Org Brain opens without a Personal Brain");
    } else if (action === "create-org-agent" || action === "create-org-human") {
      await waitForBrainClient(brain, page);
      await createOrganizationBrain(
        brain,
        expectedText,
        action === "create-org-agent",
      );
      await waitForUnlockedBrain(brain, page);
      console.log(`BRAIN_ID=${await selectedBrainId(brain)}`);
      console.log(`brain user-first ${action === "create-org-agent" ? "agent-paired" : "human-only"} Org bootstrap ok`);
    } else if (action === "create-folder") {
      await waitForUnlockedBrain(brain, page);
      await brain.locator("#obsidianNewFolderButton").click();
      await assertOwnerSeesNote(brain, slugFromFolderName(expectedText));
      console.log(`BRAIN_ID=${await selectedBrainId(brain)}`);
      console.log("brain browser-created Folder ok");
    } else if (action === "assert-folder") {
      await waitForUnlockedBrain(brain, page);
      await assertOwnerSeesNote(brain, expectedText);
      console.log(`BRAIN_ID=${await selectedBrainId(brain)}`);
      console.log("brain browser Folder readback ok");
    } else {
      await waitForUnlockedBrain(brain, page);
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

async function waitForBrainClient(brain: FrameLocator, page: Page) {
  await brain
    .locator("#sessionAccountStatus")
    .waitFor({ state: "visible", timeout: 90_000 })
    .catch(async (error) => {
      throw new Error(
        `Brain Product Client did not render: ${String(error)}\n${await page.locator("body").innerText()}`,
      );
    });
}

async function createPersonalBrain(brain: FrameLocator, agentEmail: string) {
  await openManageBrains(brain);
  const create = brain.locator("#manageCreatePersonalBrainButton");
  await create.waitFor({ state: "visible", timeout: 30_000 });
  await brain.locator("#managePersonalAgentEmailInput").fill(agentEmail);
  await create.click();
}

async function openManageBrains(brain: FrameLocator) {
  const switcher = brain.locator("#sessionAccountBrainButton");
  const manage = brain.locator("#manageBrainsButton");
  await assertEventually(
    async () => {
      if (await manage.isVisible()) return true;
      await switcher.click();
      await manage.waitFor({ state: "visible", timeout: 2_000 }).catch(() => {});
      return manage.isVisible();
    },
    30_000,
    async () => "Brain switcher did not expose Manage Brains",
  );
  await manage.click();
  await brain.locator("#manageBrainsModal").waitFor({ state: "visible", timeout: 30_000 });
}

async function closeManageBrainsIfOpen(brain: FrameLocator) {
  const modal = brain.locator("#manageBrainsModal");
  if (await modal.isVisible()) {
    await brain.locator("#closeManageBrainsButton").click();
    await modal.waitFor({ state: "hidden", timeout: 30_000 });
  }
}

async function selectedBrainId(brain: FrameLocator) {
  if (!(await brain.locator("#manageBrainsModal").isVisible())) {
    await openManageBrains(brain);
  }
  const selected = brain.locator("#manageBrainsList .brain-switch-button.selected");
  await selected.waitFor({ state: "visible", timeout: 30_000 });
  const brainId = await selected.getAttribute("data-brain-id");
  assert.ok(brainId, "Selected Brain did not expose its stable id");
  await closeManageBrainsIfOpen(brain);
  return brainId;
}

async function createOrganizationBrain(
  brain: FrameLocator,
  name: string,
  includeAgent: boolean,
) {
  await openManageBrains(brain);
  await brain.locator("#manageBrainCreateDetails").evaluate((element) => {
    (element as HTMLDetailsElement).open = true;
  });
  await brain.locator("#manageOrganizationBrainNameInput").fill(name);
  const checkbox = brain.locator("#manageOrganizationAddAgentInput");
  if ((await checkbox.isChecked()) !== includeAgent) await checkbox.click();
  await brain.locator("#manageCreateOrganizationBrainButton").click();
}

async function assertOrgFirstBrain(brain: FrameLocator, brainId: string) {
  await openManageBrains(brain);
  const selected = await brain.locator("#manageBrainsCurrentDetail").textContent();
  assert.match(selected || "", /Session unlocked/u);
  const selectedBrain = brain.locator("#manageBrainsList .brain-switch-button.selected");
  await selectedBrain.waitFor({ state: "visible", timeout: 30_000 });
  assert.equal(await brain.locator("#manageBrainsList .brain-switch-button").count(), 1);
  assert.match(
    (await selectedBrain.getAttribute("aria-label")) || "",
    /organization.*admin|admin.*organization/iu,
    `Direct target ${brainId} was not the selected admin-visible Org Brain`,
  );
  await brain.locator("#managePersonalBrainCreate").waitFor({ state: "visible" });
  assert.equal(
    await brain.locator("#manageCreatePersonalBrainButton").isDisabled(),
    false,
  );
}

async function waitForUnlockedBrain(brain: FrameLocator, page: Page) {
  const timeoutMs = Number(process.env.DEVFINITY_BRAIN_TIMEOUT_MS || 90_000);
  const status = brain.locator("#sessionAccountStatus");
  await waitForBrainClient(brain, page);
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
  await brain.locator("#closeSettingsButton").click();
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

function slugFromFolderName(value: string) {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/gu, "-")
    .replace(/^-+|-+$/gu, "");
}
