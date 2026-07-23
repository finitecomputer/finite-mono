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
    "assert-absent",
  ]);
  if (!actions.has(action)) {
    throw new Error(
      "usage: devfinity-brain-smoke.ts bootstrap|assert-existing-personal|assert-org-first|assert-note|create-org-agent|create-org-human|create-folder|assert-folder|assert-absent",
    );
  }
  if (!agentEmail.includes("@")) {
    throw new Error("DEVFINITY_BRAIN_AGENT_EMAIL must be an email");
  }
  if (["assert-note", "assert-org-first", "create-org-agent", "create-org-human", "create-folder", "assert-folder", "assert-absent"].includes(action) && !expectedText) {
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
      if (targetBrainId) await waitForUnlockedBrain(brain, page);
      else await waitForBrainClient(brain, page);
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
      await brain
        .locator("#readerFolderList .obsidian-folder-button")
        .filter({ hasText: slugFromFolderName(expectedText), visible: true })
        .first()
        .waitFor({ state: "visible", timeout: 30_000 });
      console.log(`BRAIN_ID=${await selectedBrainId(brain)}`);
      console.log("brain browser-created Folder ok");
    } else if (action === "assert-folder") {
      await waitForUnlockedBrain(brain, page);
      await assertOwnerSeesNote(brain, expectedText);
      console.log(`BRAIN_ID=${await selectedBrainId(brain)}`);
      console.log("brain browser Folder readback ok");
    } else if (action === "assert-absent") {
      await waitForUnlockedBrain(brain, page);
      await assertOwnerDoesNotSeeText(brain, expectedText);
      console.log(`BRAIN_ID=${await selectedBrainId(brain)}`);
      console.log("brain browser deletion convergence ok");
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
  const existingIds = new Set(
    await brain
      .locator("#manageBrainsList .brain-switch-button")
      .evaluateAll((buttons) =>
        buttons
          .map((button) => (button as HTMLElement).dataset.brainId || "")
          .filter(Boolean),
      ),
  );
  await brain.locator("#manageBrainCreateDetails").evaluate((element) => {
    (element as HTMLDetailsElement).open = true;
  });
  const checkbox = brain.locator("#manageOrganizationAddAgentInput");
  if ((await checkbox.isChecked()) !== includeAgent) await checkbox.click();
  await new Promise((resolve) => setTimeout(resolve, 250));
  const nameInput = brain.locator("#manageOrganizationBrainNameInput");
  await nameInput.fill(name);
  assert.equal(
    await nameInput.inputValue(),
    name,
    "Organization Brain name changed while configuring its Agent",
  );
  await brain.locator("#manageCreateOrganizationBrainButton").click();
  await assertEventually(
    async () => {
      const buttons = brain.locator("#manageBrainsList .brain-switch-button");
      if ((await buttons.count()) !== existingIds.size + 1) return false;
      const selectedId = await brain
        .locator("#manageBrainsList .brain-switch-button.selected")
        .getAttribute("data-brain-id");
      return Boolean(selectedId && !existingIds.has(selectedId));
    },
    30_000,
    async () => "Organization Brain creation did not select one new stable Brain id",
  );
}

async function assertOrgFirstBrain(brain: FrameLocator, brainId: string) {
  await openManageBrains(brain);
  const selected = await brain.locator("#manageBrainsCurrentDetail").textContent();
  assert.match(selected || "", /Session unlocked/u);
  const selectedBrain = brain.locator("#manageBrainsList .brain-switch-button.selected");
  await selectedBrain.waitFor({ state: "visible", timeout: 30_000 });
  assert.equal(
    await selectedBrain.getAttribute("data-brain-id"),
    brainId,
    "Direct target did not select the requested stable Brain id",
  );
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
  const shell = brain.locator('.obsidian-shell[data-session-status="unlocked"]');
  await waitForBrainClient(brain, page);
  await assertEventually(
    async () => shell.isVisible(),
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
  const visibleReaderMatch = brain
    .locator("#readerPageContent")
    .getByText(expectedText, { exact: false })
    .filter({ visible: true })
    .first();
  if (await visibleReaderMatch.waitFor({ state: "visible", timeout: 5_000 }).then(() => true).catch(() => false)) {
    return;
  }
  const folders = brain.locator("#readerFolderList .obsidian-folder-button");
  for (let index = 0; index < await folders.count(); index += 1) {
    const folder = folders.nth(index);
    if (!((await folder.getAttribute("class")) || "").includes("expanded")) {
      await folder.click();
    }
  }
  const page = brain
    .locator("#readerFolderList .obsidian-page-button")
    .filter({ hasText: expectedText, visible: true })
    .first();
  await page.waitFor({ state: "visible", timeout: timeoutMs });
  await page.click();
  await visibleReaderMatch
    .waitFor({ state: "visible", timeout: timeoutMs })
    .catch(async (error) => {
      throw new Error(
        `${String(error)}\nBrain content: ${(await brain.locator("body").innerText()).slice(0, 4000)}`,
      );
    });
}

async function assertOwnerDoesNotSeeText(brain: FrameLocator, expectedText: string) {
  const refresh = brain.locator("#refreshReaderButton");
  await assertEventually(
    async () => !(await refresh.isDisabled()),
    30_000,
    async () => "Brain refresh did not become available",
  );
  await refresh.click();
  await assertEventually(
    async () =>
      (await brain.locator("body").getByText(expectedText, { exact: false }).count()) === 0,
    Number(process.env.DEVFINITY_BRAIN_TIMEOUT_MS || 60_000),
    async () =>
      `Deleted Brain text remains visible: ${(await brain.locator("body").innerText()).slice(0, 4000)}`,
  );
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
