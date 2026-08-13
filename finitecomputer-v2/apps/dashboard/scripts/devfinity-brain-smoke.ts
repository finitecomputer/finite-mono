import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";

import {
  chromium,
  type Browser,
  type FrameLocator,
  type Page,
} from "playwright";

import { chromiumLaunchOptions } from "./playwright-browser";
import {
  retryProductClientBoundary,
  retryProductClientUnlock,
} from "./product-client-unlock-retry";

async function main() {
  const action = process.argv[2];
  const dashboardUrl = requiredEnv("FC_DASHBOARD_URL").replace(/\/$/u, "");
  const machineId = requiredEnv("DEVFINITY_BRAIN_MACHINE_ID");
  const runtimeContainerId = requiredEnv("DEVFINITY_BRAIN_CONTAINER_ID");
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
    "live-agent-note",
    "live-browser-folder",
    "live-agent-asset-reference",
    "reconnect-catchup",
    "live-browser-revocation",
    "live-browser-conflict",
    "live-notification-hints",
  ]);
  if (!actions.has(action)) {
    throw new Error(
      "usage: devfinity-brain-smoke.ts bootstrap|assert-existing-personal|assert-org-first|assert-note|create-org-agent|create-org-human|create-folder|assert-folder|assert-absent|live-agent-note|live-browser-folder|live-agent-asset-reference|reconnect-catchup|live-browser-revocation|live-browser-conflict|live-notification-hints",
    );
  }
  if (!agentEmail.includes("@")) {
    throw new Error("DEVFINITY_BRAIN_AGENT_EMAIL must be an email");
  }
  if (["assert-note", "assert-org-first", "create-org-agent", "create-org-human", "create-folder", "assert-folder", "assert-absent", "live-agent-note", "live-browser-folder", "live-agent-asset-reference", "reconnect-catchup", "live-browser-revocation", "live-browser-conflict", "live-notification-hints"].includes(action) && !expectedText) {
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
      if (["create-folder", "live-browser-folder"].includes(action) && dialog.type() === "prompt") {
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
    const brain = await loadBrainProductClient(
      page,
      `${dashboardUrl}/machines/${encodeURIComponent(machineId)}/brain${directBrainTarget}`,
    );

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
        page,
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
    } else if (action === "live-agent-note") {
      await waitForUnlockedBrain(brain, page);
      await writeAgentNote(runtimeContainerId, await selectedBrainId(brain), expectedText);
      await assertOwnerSeesNoteWithoutRefresh(brain, expectedText);
      console.log("brain Agent-to-browser notification convergence ok");
    } else if (action === "live-browser-folder") {
      await waitForUnlockedBrain(brain, page);
      await brain.locator("#obsidianNewFolderButton").click();
      await waitForRuntimePath(
        runtimeContainerId,
        `/data/workspace/finitebrain/${await selectedBrainId(brain)}/${slugFromFolderName(expectedText)}`,
      );
      console.log("brain browser-to-Agent notification convergence ok");
    } else if (action === "live-agent-asset-reference") {
      await waitForUnlockedBrain(brain, page);
      const brainId = await selectedBrainId(brain);
      await writeAgentAssetReference(runtimeContainerId, brainId, expectedText);
      await assertAssetReferenceOnly(
        brain,
        runtimeContainerId,
        expectedText,
      );
      console.log("brain Asset Source Note synced without inline bytes ok");
    } else if (action === "reconnect-catchup") {
      await waitForUnlockedBrain(brain, page);
      await assertOwnerSeesNoteWithoutRefresh(brain, expectedText);
      console.log("brain reconnect authoritative-sequence catch-up ok");
    } else if (action === "live-browser-revocation") {
      await waitForUnlockedBrain(brain, page);
      await assertOwnerSeesNoteWithoutRefresh(brain, expectedText);
      const brainId = await selectedBrainId(brain);
      revokeBrowserActor(runtimeContainerId, brainId);
      await assertEventually(
        async () => {
          await openManageBrains(brain);
          const visibleIds = await brain
            .locator("#manageBrainsList .brain-switch-button")
            .evaluateAll((buttons) =>
              buttons.map((button) => (button as HTMLElement).dataset.brainId || ""),
            );
          await closeManageBrainsIfOpen(brain);
          return !visibleIds.includes(brainId);
        },
        30_000,
        async () => "revoked Organization Brain remained visible in the browser",
      );
      assert.equal(
        (await brain.locator("#readerPageContent").textContent())?.includes(expectedText),
        false,
        "revoked Brain plaintext remained in the active browser projection",
      );
      console.log("brain browser revocation cleared its active decrypted projection ok");
    } else if (action === "live-browser-conflict") {
      await waitForUnlockedBrain(brain, page);
      await assertOwnerSeesNoteWithoutRefresh(brain, expectedText);
      await brain.locator("#editorDrawer").evaluate((node) => {
        (node as HTMLDetailsElement).open = true;
      });
      const localDraft = `${expectedText} browser draft`;
      await brain.locator("#pageDraftInput").fill(`# Browser draft\n\n${localDraft}\n`);
      const brainId = await selectedBrainId(brain);
      writeAgentConflict(runtimeContainerId, brainId, expectedText);
      const unrelated = brain
        .locator("#readerFolderList .obsidian-page-button")
        .filter({ hasText: `${expectedText} unrelated`, visible: true })
        .first();
      await unrelated.waitFor({ state: "visible", timeout: 60_000 });
      assert.ok((await brain.locator("#pageDraftInput").inputValue()).includes(localDraft));
      assert.ok(
        dockerExec(runtimeContainerId, [
          "grep",
          "-F",
          `${expectedText} remote`,
          `/data/workspace/finitebrain/${brainId}/matrix-revocation/browser-revocation.md`,
        ]).includes(`${expectedText} remote`),
        "remote conflicting version was not recoverable from the authoritative Agent projection",
      );
      console.log("brain browser conflict preserved its draft while unrelated progress converged ok");
    } else if (action === "live-notification-hints") {
      await waitForUnlockedBrain(brain, page);
      await assertOwnerSeesNoteWithoutRefresh(brain, expectedText);
      const brainId = await selectedBrainId(brain);
      let reconcileRequests = 0;
      page.on("request", (request) => {
        if (
          new URL(request.url()).pathname.endsWith(
            `/v1/brains/${encodeURIComponent(brainId)}/metadata`,
          )
        ) reconcileRequests += 1;
      });
      await brain.locator("body").evaluate(async (_body, activeBrainId) => {
        const api = (window as typeof window & {
          FiniteBrainProductClient: {
            applyBrainUpdateNotification(value: unknown): Promise<void>;
          };
        }).FiniteBrainProductClient;
        const duplicate = {
          brainId: activeBrainId,
          latestSequence: Number.MAX_SAFE_INTEGER,
          reason: "content_updated",
        };
        await Promise.all([
          api.applyBrainUpdateNotification(duplicate),
          api.applyBrainUpdateNotification(duplicate),
          api.applyBrainUpdateNotification({ ...duplicate }),
        ]);
      }, brainId);
      await new Promise((resolve) => setTimeout(resolve, 750));
      assert.equal(
        reconcileRequests,
        1,
        "same-Brain duplicate hint burst must coalesce into one reconciliation",
      );
      await assertOwnerSeesNoteWithoutRefresh(brain, expectedText);
      console.log("brain duplicate notification tolerance and same-Brain coalescing ok");
    } else {
      await waitForUnlockedBrain(brain, page);
      await assertOwnerSeesNote(brain, expectedText);
      console.log("brain owner readback ok");
    }

    await context.close();
  } catch (error) {
    try {
      const supervisorLog = dockerExec(runtimeContainerId, [
        "/bin/bash",
        "-lc",
        "test ! -f /tmp/fbrain-supervisor.log || tail -200 /tmp/fbrain-supervisor.log",
      ]).trim();
      if (supervisorLog) diagnostics.push(`fbrain supervisor:\n${supervisorLog}`);
    } catch (diagnosticError) {
      diagnostics.push(
        `fbrain supervisor diagnostics unavailable: ${diagnosticError instanceof Error ? diagnosticError.message : String(diagnosticError)}`,
      );
    }
    const detail = diagnostics.length ? `\n${diagnostics.join("\n")}` : "";
    throw new Error(
      `${error instanceof Error ? error.message : String(error)}${detail}`,
    );
  } finally {
    await browser?.close().catch(() => {});
  }
}

function dockerExec(machineId: string, args: string[]): string {
  return execFileSync("docker", ["exec", machineId, ...args], {
    encoding: "utf8",
    timeout: 60_000,
  });
}

function revokeBrowserActor(machineId: string, brainId: string) {
  const script = [
    "set -euo pipefail",
    'agent="$(fbrain signer public-key)"',
    'target="$(fbrain brain metadata "$MATRIX_BRAIN_ID" --json | python3 -c \'import json,sys; agent=sys.argv[1]; print(next(value for value in json.load(sys.stdin).get("admins", []) if value != agent))\' "$agent")"',
    'fbrain admin role revoke admin --brain "$MATRIX_BRAIN_ID" --target "$target" --json >/dev/null',
    'fbrain admin member remove --brain "$MATRIX_BRAIN_ID" --target "$target" --json >/dev/null',
  ].join("\n");
  dockerExec(machineId, [
    "env",
    `MATRIX_BRAIN_ID=${brainId}`,
    "/bin/bash",
    "-lc",
    script,
  ]);
}

function writeAgentConflict(machineId: string, brainId: string, marker: string) {
  dockerExec(machineId, [
    "env",
    `MATRIX_BRAIN_ID=${brainId}`,
    `MATRIX_CONFLICT_MARKER=${marker}`,
    "/bin/bash",
    "-lc",
    [
      "set -euo pipefail",
      'root="/data/workspace/finitebrain/$MATRIX_BRAIN_ID/matrix-revocation"',
      'printf "# %s remote\\n\\n%s\\n" "$MATRIX_CONFLICT_MARKER" "$MATRIX_CONFLICT_MARKER" >"$root/browser-revocation.md"',
      'printf "# %s unrelated\\n\\nMust converge around the browser draft.\\n" "$MATRIX_CONFLICT_MARKER" >"$root/unrelated-progress.md"',
    ].join("\n"),
  ]);
}

async function writeAgentNote(machineId: string, brainId: string, marker: string) {
  dockerExec(machineId, [
    "env",
    `MATRIX_MARKER=${marker}`,
    "/bin/bash",
    "-lc",
    `printf '# %s\\n\\nNotification-driven Agent edit.\\n' "$MATRIX_MARKER" > /data/workspace/finitebrain/${brainId}/agent-notes/devfinity-agent-proof.md`,
  ]);
}

async function writeAgentAssetReference(
  machineId: string,
  brainId: string,
  filename: string,
) {
  const noteRoot = `/data/workspace/finitebrain/${brainId}/agent-notes`;
  const assetRoot = "/data/workspace/asset-sources";
  dockerExec(machineId, [
    "env",
    `MATRIX_ASSET_FILENAME=${filename}`,
    `MATRIX_ASSET_ROOT=${assetRoot}`,
    `MATRIX_BRAIN_NOTE_ROOT=${noteRoot}`,
    "/bin/bash",
    "-lc",
    [
      "set -euo pipefail",
      'asset="$MATRIX_ASSET_ROOT/$MATRIX_ASSET_FILENAME"',
      'note="$MATRIX_BRAIN_NOTE_ROOT/raw/matrix-asset.md"',
      'mkdir -p "$MATRIX_ASSET_ROOT" "$MATRIX_BRAIN_NOTE_ROOT/raw"',
      "printf '\\000FiniteBrain\\377Asset\\n' >\"$asset\"",
      'cat >"$note" <<EOF',
      "---",
      "type: asset",
      'title: "Matrix Asset Reference: $MATRIX_ASSET_FILENAME"',
      'resource: "file://$asset"',
      "finite_asset:",
      "  content_type: application/octet-stream",
      "---",
      "# Matrix Asset Reference: $MATRIX_ASSET_FILENAME",
      "",
      "The binary remains at [its canonical local resource](file://$asset).",
      "EOF",
    ].join("\n"),
  ]);
}

async function waitForRuntimePath(machineId: string, path: string) {
  dockerExec(machineId, [
    "/bin/bash",
    "-lc",
    `for attempt in $(seq 1 300); do test -e '${path}' && exit 0; sleep 0.1; done; echo 'timed out waiting for ${path}' >&2; exit 1`,
  ]);
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

async function loadBrainProductClient(page: Page, url: string): Promise<FrameLocator> {
  let lastError: unknown = null;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    const configLoaded = page
      .waitForResponse(
        (response) => {
          try {
            return new URL(response.url()).pathname === "/client/config.json"
              && response.ok();
          } catch {
            return false;
          }
        },
        { timeout: 20_000 },
      )
      .then(() => true, () => false);
    try {
      await page.goto(url, {
        waitUntil: "domcontentloaded",
        timeout: 60_000,
      });
      const brainFrame = page.locator('iframe[title$=" Brain"]');
      await brainFrame.waitFor({ state: "visible", timeout: 60_000 });
      assert.match(
        (await brainFrame.getAttribute("sandbox")) || "",
        /(?:^|\s)allow-modals(?:\s|$)/u,
        "Hosted Brain frame must permit its bounded Product Client dialogs",
      );
      const brain = page.frameLocator('iframe[title$=" Brain"]');
      await waitForBrainClient(brain, page);
      if (!(await configLoaded)) {
        throw new Error("Product Client config request did not succeed");
      }
      return brain;
    } catch (error) {
      lastError = error;
      await configLoaded;
      if (attempt < 3) await page.goto("about:blank");
    }
  }
  throw new Error(
    `Brain Product Client config did not load after a dependency restart: ${String(lastError)}`,
  );
}

async function createPersonalBrain(brain: FrameLocator, agentEmail: string) {
  await openManageBrains(brain);
  const create = brain.locator("#manageCreatePersonalBrainButton");
  const connectSigner = brain.locator("#manageBrainsConnectSignerButton");
  await create.waitFor({ state: "visible", timeout: 30_000 });
  await brain.locator("#managePersonalAgentEmailInput").fill(agentEmail);

  const timeoutMs = 30_000;
  const deadline = Date.now() + timeoutMs;
  const remaining = () => Math.max(0, deadline - Date.now());
  const createReadyMessage =
    "Personal Brain Create stayed disabled because signer, config, or readerBusy was not ready";
  const createReadyOrSignerConnected = async () =>
    (await create.isEnabled()) || !(await connectSigner.isVisible());

  await assertEventually(
    createReadyOrSignerConnected,
    remaining(),
    async () => createReadyMessage,
  );
  if (!(await create.isEnabled())) {
    if (remaining() <= 0) throw new Error(createReadyMessage);
    await closeManageBrainsIfOpen(brain);
    await openManageBrains(brain);
    await brain.locator("#managePersonalAgentEmailInput").fill(agentEmail);
    await create.waitFor({ state: "visible", timeout: remaining() });
    await assertEventually(
      async () => create.isEnabled(),
      remaining(),
      async () => createReadyMessage,
    );
  }
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
  page: Page,
  name: string,
  includeAgent: boolean,
) {
  const timeoutMs = Number(process.env.DEVFINITY_BRAIN_TIMEOUT_MS || 90_000);
  const url = page.url();
  const priorBrainIds = await retryProductClientBoundary({
    timeoutMs,
    attempt: async (attemptTimeoutMs) =>
      prepareOrganizationBrain(
        brain,
        name,
        includeAgent,
        attemptTimeoutMs,
      ),
    reload: async () => {
      await loadBrainProductClient(page, url);
    },
  });
  await brain.locator("#manageCreateOrganizationBrainButton").click();
  await assertEventually(
    async () => {
      const buttons = brain.locator("#manageBrainsList .brain-switch-button");
      if ((await buttons.count()) !== priorBrainIds.size + 1) return false;
      const selectedId = await brain
        .locator("#manageBrainsList .brain-switch-button.selected")
        .getAttribute("data-brain-id");
      return Boolean(selectedId && !priorBrainIds.has(selectedId));
    },
    30_000,
    async () => "Organization Brain creation did not select one new stable Brain id",
  );
}

async function prepareOrganizationBrain(
  brain: FrameLocator,
  name: string,
  includeAgent: boolean,
  timeoutMs: number,
): Promise<Set<string>> {
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
  const create = brain.locator("#manageCreateOrganizationBrainButton");
  await assertEventually(
    async () => create.isEnabled(),
    timeoutMs,
    async () => "Organization Brain Create action did not become ready",
  );
  return existingIds;
}

async function assertOrgFirstBrain(brain: FrameLocator, brainId: string) {
  await openManageBrains(brain);
  assert.equal(
    await brain.locator(".obsidian-shell").getAttribute("data-session-status"),
    "unlocked",
  );
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
  const url = page.url();
  await retryProductClientUnlock({
    timeoutMs,
    waitForUnlock: async (attemptTimeoutMs) => {
      await waitForBrainClient(brain, page);
      await assertEventually(
        async () => shell.isVisible(),
        attemptTimeoutMs,
        async () =>
          `Brain did not unlock; current status: ${(await status.textContent())?.trim()}`,
      );
    },
    reload: async () => {
      await loadBrainProductClient(page, url);
    },
  });
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

async function assertOwnerSeesNoteWithoutRefresh(brain: FrameLocator, expectedText: string) {
  const timeoutMs = Number(process.env.DEVFINITY_BRAIN_TIMEOUT_MS || 60_000);
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
  await brain
    .locator("#readerPageContent")
    .getByText(expectedText, { exact: false })
    .filter({ visible: true })
    .first()
    .waitFor({ state: "visible", timeout: timeoutMs });
}

async function assertAssetReferenceOnly(
  brain: FrameLocator,
  machineId: string,
  filename: string,
) {
  await assertOwnerSeesNoteWithoutRefresh(brain, filename);

  const inlineAsset = brain
    .locator("#readerFolderList .obsidian-page-button.asset")
    .filter({ hasText: filename, visible: true });
  assert.equal(
    await inlineAsset.count(),
    0,
    "non-Markdown bytes appeared as an inline Brain Asset",
  );
  assert.equal(
    await brain.locator("#readerPageContent .asset-download-button").count(),
    0,
    "Asset Source Note exposed an inline Brain download",
  );

  const assetPath = `/data/workspace/asset-sources/${filename}`;
  const digest = dockerExec(machineId, [
    "sha256sum",
    assetPath,
  ]).split(/\s/u)[0];
  assert.equal(
    digest,
    "afcb8f2e722009f83487834905e0d459c8579e6262ca77ba1c633277ca157e09",
    "local Asset bytes changed or disappeared during reference sync",
  );
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
