import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { existsSync } from "node:fs";
import http, { type IncomingMessage, type ServerResponse } from "node:http";
import { once } from "node:events";
import { test } from "node:test";

import { chromium, type Browser, type Page } from "playwright";

const CORE_TOKEN = "browser-core-token";
const HOSTED_DEVICE_TOKEN = "browser-hosted-device-token";
const SITES_VIEWER_SESSION_TOKEN = "browser-sites-viewer-session-token";
const AGENT_NPUB = "npub1browseragentprincipal";
const AGENT_PICTURE_URL = "https://chat.example/blobs/browser-agent-picture.png";
const PNG_BYTES = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
  "base64"
);

type AgentCreationRequest = {
  id: string;
  project_id: string;
  display_name: string;
  runner_class: "apple_container" | "kata";
  profile_picture_url: string | null;
  status: "requested" | "launching" | "running" | "failed" | "cancelled";
  agent_runtime_id: string | null;
  failure_message: string | null;
  created_at: string;
  updated_at: string;
};

type VisibleProject = {
  project: {
    id: string;
    customer_org_id: string;
    owner_user_id: string;
    display_name: string;
    import_candidate_id: string | null;
    created_at: string;
    updated_at: string;
  };
  runtime: null | {
    id: string;
    project_id: string;
    source_host_id: string;
    source_machine_id: string;
    source_import_key: string;
    runtime_artifact_id: string | null;
    state_schema_version: string | null;
    host_facts: {
      display_name: string;
      hostname: string;
      runtime_host: string;
      runtime_status: "online" | "offline" | "stale" | "unknown";
      hermes_available: boolean;
      published_app_urls: string[];
    };
    created_at: string;
    updated_at: string;
  };
};

type CoreState = {
  projects: VisibleProject[];
  requests: AgentCreationRequest[];
  creationPosts: unknown[];
  cancelPosts: string[];
  destroyPosts: string[];
  createDelayMs: number;
  canCreateAgent: boolean;
  creationError: string | null;
};

type FakeHostedChatState = {
  rev: number;
  identity: {
    account_id: string;
    device_id: string;
  };
  rooms: Array<{
    room_id: string;
    display_name: string;
    state: "Connected";
    status: string;
    user_status_text: string;
    last_message_preview: string;
    unread_count: number;
    is_agent_chat: boolean;
  }>;
  selected_room_id: string | null;
  topics: Array<{
    room_id: string;
    topic_id: string;
    title: string;
    active_chat_id: string | null;
    chats: Array<{
      chat_id: string;
      title: string;
      active: boolean;
    }>;
  }>;
  selected_topic_id: string | null;
  selected_chat_id: string | null;
  active_profile_id: string | null;
  status: string;
  toast: null;
  messages: Array<{
    room_id: string;
    seq: number;
    message_id: string;
    conversation_id: string;
    chat_id: string;
    sender_account_id: string;
    sender_display_name: string;
    text: string;
    display_content: string;
    kind: "message" | "status" | "tool" | "media";
    status: "running" | "complete";
    final_delivery: boolean;
    edit_of_message_id: string | null;
    is_mine: boolean;
    media: Array<{
      attachment_id: string;
      mime_type: string;
      filename: string;
      kind: "Image" | "File";
      width: number | null;
      height: number | null;
    }>;
    timestamp_unix_seconds: number;
    display_timestamp: string;
  }>;
  typing_members: Array<{
    room_id: string;
    topic_id: string | null;
    chat_id: string | null;
    account_id: string;
    device_id: string;
    display_name: string;
    activity_kind: "typing" | "thinking" | "working";
  }>;
  profiles: Array<{
    account_id: string;
    npub: string;
    display_name: string;
    about: string | null;
    picture: string | null;
    stale: boolean;
    is_agent: boolean;
  }>;
  flow: {
    notice_text: string | null;
    notice_busy: boolean;
    scan_in_flight: boolean;
    scan_result: string;
  };
};

type HostedAuthRequest = {
  method: string;
  path: string;
  authorization: string | null;
  workosUserId: string | null;
};

type HostedDeviceState = {
  app: FakeHostedChatState;
  actions: Array<Record<string, unknown>>;
  runtimeCommands: Array<Record<string, unknown>>;
  authRequests: HostedAuthRequest[];
  connections: AgentConnectionsStatus;
};

type AgentConnectionsStatus = {
  inference: {
    profile: "finite_private" | "openrouter";
    provider: string;
    model: string;
  };
  telegram: {
    connected: boolean;
    home_channel: string | null;
    pending: Array<{ user_id: string; name: string }>;
    approved: Array<{ user_id: string; name: string }>;
  };
  google: {
    connected: boolean;
    email: string | null;
  };
};

type FakeSitesState = {
  exchanges: Array<{
    authorization: string | null;
    outputUrl: string;
    verifiedEmail: string;
    returnTo: string;
  }>;
  redemptions: number;
  privateContentRequests: number;
};

test("dashboard agent creation browser states", { timeout: 120_000 }, async () => {
  const hostedDevice = await startFakeHostedDevice();
  const core = await startFakeCore();
  const brain = await startFakeBrain();
  const sites = await startFakeSites();
  const dashboardPort = await freePort();
  const dashboard = startDashboard(
    dashboardPort,
    core.url,
    hostedDevice.url,
    brain.url,
    sites.apiUrl
  );
  const dashboardOutput = collectOutput(dashboard);
  let browser: Browser | null = null;

  try {
    await waitForDashboard(dashboardPort, dashboardOutput);
    browser = await chromium.launch({
      headless: true,
      ...chromeExecutable(),
    });

    core.reset();
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      const agentName = page.getByLabel("Agent name");
      await agentName.waitFor({ state: "visible", timeout: 15_000 }).catch(async (error) => {
        throw new Error(
          `Agent creation form did not render: ${String(error)}\n${await pageText(page)}\n${dashboardOutput()}`
        );
      });
      await page.getByRole("link", { name: "New agent", exact: true }).waitFor({
        state: "visible",
      });
      await agentName.fill("Oslo Bot");
      await page.locator('#coreAgentPicture').setInputFiles({
        name: "oslo-bot.png",
        mimeType: "image/png",
        buffer: PNG_BYTES,
      });
      await page.getByRole("img", { name: "Agent profile preview" }).waitFor({ state: "visible" });
      await page.getByRole("button", { name: "Continue" }).click();
      await page.getByLabel("Launch Code").fill("fixture-launch-code");
      await page.getByRole("button", { name: "Apply" }).click();
      await waitFor(
        () => core.state.creationPosts.length === 1,
        5_000,
        async () => `agent creation POST was not sent\n${await pageText(page)}`
      );
      const post = core.state.creationPosts[0] as Record<string, unknown>;
      assert.equal(post.displayName, "Oslo Bot");
      assert.equal(post.launchCode, "fixture-launch-code");
      assert.equal(post.runnerClass, "apple_container");
      assert.equal(post.profilePictureUrl, AGENT_PICTURE_URL);
      assert.match(String(post.idempotencyKey), /.+/);
      await page.waitForURL(/\/dashboard\?new=1&creation=agent_request_1$/u);
      await expectVisibleText(page, "Creating your agent");
      assert(
        hostedDevice.state.authRequests.some((request) => request.path === "/v1/app/images"),
        "agent picture did not use the authenticated Hosted Device upload"
      );
    });

    core.reset({ createDelayMs: 500 });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await page.getByLabel("Agent name").fill("Double Submit Bot");
      await page.getByRole("button", { name: "Continue" }).click();
      await page.getByLabel("Launch Code").fill("fixture-launch-code");
      await page.getByRole("button", { name: "Apply" }).dblclick();
      await waitFor(() => core.state.creationPosts.length === 1);
      await new Promise((resolve) => setTimeout(resolve, 700));
      assert.equal(core.state.creationPosts.length, 1);
    });

    core.reset({
      canCreateAgent: true,
      creationError: "billing is required before creating an agent",
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard?new=1`);
      await page.getByLabel("Agent name").fill("Fresh Code Bot");
      await page.getByRole("button", { name: "Continue" }).click();
      await page.getByLabel("Launch Code").fill("fresh-top-up-code");
      await page.getByRole("button", { name: "Apply" }).click();
      await expectVisibleText(page, "billing is required before creating an agent");
      await new Promise((resolve) => setTimeout(resolve, 750));
      assert.equal(
        core.state.creationPosts.length,
        1,
        "a failed launch must return to the form instead of resubmitting the saved draft"
      );
      assert.equal(
        (core.state.creationPosts[0] as Record<string, unknown>).launchCode,
        "fresh-top-up-code",
        "an explicitly submitted Launch Code must win over stale entitlement capacity"
      );
    });

    core.reset({
      requests: [
        agentCreationRequest({
          id: "agent_request_waiting",
          projectId: "project_waiting",
          displayName: "Queued Oslo Bot",
          status: "requested",
          createdAt: "2026-05-28T12:00:00Z",
        }),
      ],
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await expectVisibleText(page, "Waiting for runner capacity");
      await expectVisibleText(
        page,
        "All runner hosts are busy. Your agent is still queued and will start automatically when capacity opens."
      );
    });

    core.reset({
      requests: [
        agentCreationRequest({
          id: "agent_request_failed",
          projectId: "project_failed",
          displayName: "Failed Oslo Bot",
          status: "failed",
          failureMessage: "Runner capacity exhausted",
        }),
      ],
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await expectVisibleText(page, "Agent creation needs a retry");
      await expectVisibleText(page, "Runner capacity exhausted");
      await page.getByRole("button", { name: "Start over" }).click();
      await waitFor(() => core.state.cancelPosts.includes("agent_request_failed"));
    });

    core.reset({
      projects: [
        visibleProject(
          "project_running",
          "Completed Oslo Bot",
          hostedDevice.runtimeStatusUrl
        ),
        visibleProject(
          "project_second",
          "Second Oslo Bot",
          hostedDevice.runtimeStatusUrl,
          "second-oslo-bot"
        ),
      ],
      requests: [
        agentCreationRequest({
          id: "agent_request_old",
          projectId: "project_running",
          displayName: "Completed Oslo Bot",
          status: "running",
        }),
        agentCreationRequest({
          id: "agent_request_second",
          projectId: "project_second",
          displayName: "Second Oslo Bot",
          status: "running",
        }),
      ],
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await page.waitForURL(/\/dashboard\/machines\/completed-oslo-bot$/u);
      await page
        .getByRole("heading", { name: "Completed Oslo Bot", exact: true })
        .waitFor({ state: "visible" });
      const main = page.getByRole("main");
      await expectVisibleText(page, "Your agent is online.");
      const productNav = page.getByRole("navigation", { name: "Dashboard section" });
      const agentNav = productNav.getByRole("link", { name: "Agent", exact: true });
      await agentNav.waitFor({ state: "visible" });
      // The fake Core changes out of band, unlike a product mutation that
      // invalidates the dashboard's short SWR cache. A stale server render
      // starts the background refresh; reload until that refreshed projection
      // is visible instead of racing one fixed sleep against CI load.
      await waitFor(
        async () => {
          if ((await agentNav.getAttribute("aria-current")) === "page") {
            return true;
          }
          await page.waitForTimeout(250);
          await page.reload();
          return (await agentNav.getAttribute("aria-current")) === "page";
        },
        15_000,
        async () => `agent navigation did not hydrate from Core\n${await pageText(page)}`
      );
      await productNav.getByRole("link", { name: "Connections", exact: true }).waitFor({ state: "visible" });
      assert.equal(await productNav.getByRole("link", { name: "Brain", exact: true }).count(), 0);
      assert.equal(await productNav.getByRole("link", { name: "Skills", exact: true }).count(), 0);
      await productNav.getByRole("link", { name: "Chat", exact: true }).waitFor({ state: "visible" });

      const machineSwitcher = page.getByRole("button", {
        name: "Completed Oslo Bot",
        exact: true,
      });
      await machineSwitcher.waitFor({ state: "visible" });
      await machineSwitcher.click();
      const newAgentItem = page.getByRole("menuitem", { name: "New agent", exact: true });
      await newAgentItem.waitFor({ state: "visible" });
      await newAgentItem.click();
      await page.waitForURL(/\/dashboard\?new=1$/u);
      await page.getByLabel("Agent name").waitFor({ state: "visible" });
      assert.equal(await page.getByText("Completed Oslo Bot", { exact: true }).count(), 0);
      await page.getByLabel("Agent name").fill("Second Oslo Bot");
      await page.getByRole("button", { name: "Continue" }).click();
      await page.getByLabel("Launch Code").waitFor({ state: "visible" });

      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard?new=1&creation=agent_request_second`
      );
      await page.waitForURL(/\/dashboard\/machines\/second-oslo-bot$/u);

      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/completed-oslo-bot`
      );
      await main.getByRole("button", { name: "Restart agent" }).waitFor({ state: "visible" });
      assert.equal(await main.getByRole("button", { name: "Recover chat" }).count(), 0);
      await main.getByRole("button", { name: "Stop" }).waitFor({ state: "visible" });
      assert.equal(await main.getByRole("button", { name: "Destroy" }).count(), 0);
      const openWebChat = main.getByRole("link", { name: "Open chat" });
      await openWebChat.waitFor({ state: "visible" });

      await openWebChat.click();
      await page.waitForURL(/\/dashboard\/machines\/completed-oslo-bot\/chat$/u);
      await expectVisibleText(page, "Hello from Completed Oslo Bot.");
      await expectVisibleText(page, "Topics");
      await page
        .getByRole("button", { name: "New chat in General", exact: true })
        .waitFor({ state: "visible" });
      await page
        .getByRole("button", { name: "New chat", exact: true })
        .waitFor({ state: "visible" });
      await expectVisibleText(page, "browser@finite.vip");
      assert(
        hostedDevice.state.runtimeCommands.some(
          (command) => command.command === "agent.owner.claim"
        ),
        "Chat became usable before the owner claim succeeded"
      );
      assert.equal(await page.getByRole("link", { name: "Finite.Computer" }).count(), 0);
      await page.getByRole("link", { name: "Connections" }).click();
      await page.waitForURL(/\/dashboard\/machines\/completed-oslo-bot\/connections$/u);
      await expectVisibleText(page, "Finite Private · openai/gpt-oss-120b");
      await expectVisibleText(page, "Google Workspace");
      const ownerClaimIndex = hostedDevice.state.runtimeCommands.findIndex(
        (command) => command.command === "agent.owner.claim"
      );
      const connectionsStatusIndex = hostedDevice.state.runtimeCommands.findIndex(
        (command) => command.command === "agent.connections.status"
      );
      assert(ownerClaimIndex >= 0, "Chat/Connections became usable without an owner claim");
      assert(
        connectionsStatusIndex > ownerClaimIndex,
        "Connections status was requested before the owner claim succeeded"
      );
      await page.getByRole("button", { name: "Use OpenRouter" }).click();
      await page.getByLabel("OpenRouter key").fill("test-only-invalid-key");
      await page.getByLabel("OpenRouter model").fill("openai/gpt-5-mini");
      await page.getByRole("button", { name: "Save" }).click();
      await expectVisibleText(page, "OpenRouter · openai/gpt-5-mini");
      assert(
        hostedDevice.state.runtimeCommands.some(
          (command) => command.command === "agent.inference.apply"
        ),
        "inference change did not use the runtime command channel"
      );

      await page.getByRole("main").evaluate((element) => {
        element.scrollTop = 120;
      });
      assert.equal(
        await page.getByRole("link", { name: "Brain", exact: true }).count(),
        0,
        "Brain must remain hidden from canary navigation"
      );
      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/completed-oslo-bot/brain`
      );
      await waitFor(async () => (await page.getByRole("main").evaluate((element) => element.scrollTop)) === 0);
      const brainFrame = page.frameLocator('iframe[title="Completed Oslo Bot Brain"]');
      await brainFrame.getByText("FiniteBrain browser proof").waitFor({ state: "visible" });
      await brainFrame.getByText("Brain API ready").waitFor({ state: "visible" });
      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/completed-oslo-bot/chat`
      );

      await page.getByRole("button", { name: "Rename chat" }).click();
      const renameDialog = page.getByRole("dialog", { name: "Rename chat" });
      await renameDialog.getByRole("textbox", { name: "Name" }).fill("Browser QA");
      await renameDialog.getByRole("button", { name: "Save" }).click();
      await waitFor(() =>
        hostedDevice.state.actions.some(
          (action) => actionName(action) === "RenameChat"
        )
      );
      await page
        .getByRole("banner")
        .getByText("Browser QA", { exact: true })
        .waitFor({ state: "visible" });

      const bootstrapActions = hostedDevice.state.actions.map(actionName);
      const startRuntimeIndex = bootstrapActions.indexOf("StartRuntime");
      const scanTargetIndex = bootstrapActions.indexOf("ScanTarget");
      const startProfileChatIndex = bootstrapActions.indexOf("StartProfileChat");
      assert(startRuntimeIndex >= 0);
      assert(scanTargetIndex > startRuntimeIndex);
      assert(startProfileChatIndex > scanTargetIndex);

      const message = "Keep the runtime boundary thin.";
      await page.getByLabel("Message your agent").fill(message);
      await page.getByRole("button", { name: "Send message" }).click();
      await waitFor(() =>
        hostedDevice.state.actions.some(
          (action) => actionName(action) === "SendChatMessage"
        )
      );
      await page
        .getByRole("paragraph")
        .filter({ hasText: message })
        .waitFor({ state: "visible", timeout: 15_000 });

      const sendAction = hostedDevice.state.actions.find(
        (action) => actionName(action) === "SendChatMessage"
      );
      assert.deepEqual(sendAction, {
        SendChatMessage: {
          room_id: "room_browser_agent",
          topic_id: "topic_browser_agent",
          chat_id: "chat_browser_agent",
          text: message,
        },
      });
      assert.equal(
        hostedDevice.state.app.messages.some(
          (candidate) => candidate.is_mine && candidate.text === message
        ),
        true
      );

      await page.locator('input[type="file"]').setInputFiles({
        name: "browser-proof.png",
        mimeType: "image/png",
        buffer: PNG_BYTES,
      });
      await page.getByLabel("Message your agent").fill("Image from browser.");
      await page.getByRole("button", { name: "Send message" }).click();
      await page.getByRole("img", { name: "browser-proof.png" }).waitFor({ state: "visible" });

      hostedDevice.state.app.messages.push(
        hostedImageMessage("Image returned by agent.", false, 4, "agent-proof.png")
      );
      hostedDevice.emit();
      await page.getByRole("img", { name: "agent-proof.png" }).waitFor({ state: "visible" });
      await waitFor(() =>
        hostedDevice.state.authRequests.some((request) =>
          request.path.startsWith("/v1/app/attachments/")
        )
      );

      hostedDevice.state.app.typing_members = [
        {
          room_id: "room_browser_agent",
          topic_id: "topic_browser_agent",
          chat_id: "chat_browser_agent",
          account_id: "agent-account-browser",
          device_id: "agent",
          display_name: "Completed Oslo Bot",
          activity_kind: "working",
        },
      ];
      hostedDevice.emit();
      await expectVisibleText(page, "Completed Oslo Bot is working");

      hostedDevice.state.app.typing_members = [];
      hostedDevice.state.app.messages.push({
        ...hostedMessage("💻 Running browser QA", false, 5),
        kind: "tool",
        status: "running",
      });
      hostedDevice.emit();
      await expectVisibleText(page, "Working · 1 step");
      await expectVisibleText(page, "Completed Oslo Bot is working");

      hostedDevice.state.app.messages[hostedDevice.state.app.messages.length - 1]!.status =
        "complete";
      hostedDevice.state.app.messages.push({
        ...hostedMessage("Browser QA complete.", false, 6),
        final_delivery: true,
      });
      hostedDevice.emit();
      await expectVisibleText(page, "Worked through 1 step");
      await page
        .getByText("Completed Oslo Bot is working", { exact: true })
        .waitFor({ state: "hidden", timeout: 15_000 });

      const localSiteUrl = sites.siteUrl;
      hostedDevice.state.app.messages.push(
        hostedMessage("Repository: https://git.finite.chat/browser-proof.git", false, 7)
      );
      hostedDevice.state.app.messages.push(
        hostedMessage(`Published your site: ${localSiteUrl}`, false, 8)
      );
      hostedDevice.emit();
      await page.getByRole("button", { name: "Preview" }).click();
      await page.getByLabel("Preview URL").waitFor({ state: "visible" });
      assert.equal(await page.getByLabel("Preview URL").inputValue(), localSiteUrl);
      assert.equal(await page.getByLabel("Select site preview").count(), 0);
      const siteFrame = page.frameLocator('iframe[title="browser-proof.sites.localhost"]');
      await siteFrame.getByText("Private site browser proof").waitFor({
        state: "visible",
        timeout: 15_000,
      });
      assert(sites.state.exchanges.length >= 1);
      for (const exchange of sites.state.exchanges) {
        assert.deepEqual(exchange, {
          authorization: `Bearer ${SITES_VIEWER_SESSION_TOKEN}`,
          outputUrl: localSiteUrl,
          verifiedEmail: "browser@finite.vip",
          returnTo: "/",
        });
      }
      assert.equal(sites.state.redemptions, 1);
      assert(sites.state.privateContentRequests >= 1);
      assert.match(
        (await page.getByLabel("Site preview").locator("iframe").getAttribute("src")) ?? "",
        /\/_finite\/auth\?token=/u
      );

      await waitFor(() =>
        hostedDevice.state.authRequests.some(
          (request) => request.path === "/v1/app/updates"
        )
      );
      const hostedPaths = new Set(hostedDevice.state.authRequests.map((request) => request.path));
      for (const requiredPath of ["/v1/app/state", "/v1/app/actions", "/v1/app/updates", "/v1/app/attachments"]) {
        assert(hostedPaths.has(requiredPath), requiredPath);
      }
      assert(
        [...hostedPaths].some((path) => path.startsWith("/v1/app/attachments/")),
        "authenticated image download did not reach the Hosted Device"
      );
      for (const request of hostedDevice.state.authRequests) {
        assert.equal(request.authorization, `Bearer ${HOSTED_DEVICE_TOKEN}`);
        assert.equal(request.workosUserId, "user_browser");
      }
    });

    core.reset({
      projects: [
        visibleProject(
          "project_removable",
          "Removable Kata Bot",
          hostedDevice.runtimeStatusUrl,
          "removable-kata-bot"
        ),
      ],
      requests: [
        agentCreationRequest({
          id: "agent_request_removable",
          projectId: "project_removable",
          displayName: "Removable Kata Bot",
          status: "running",
          runnerClass: "kata",
          agentRuntimeId: "runtime_removable-kata-bot",
        }),
      ],
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/removable-kata-bot`
      );
      const removeButton = page.getByRole("button", { name: "Remove agent" });
      await removeButton.waitFor({ state: "visible" });
      const removeForm = removeButton.locator("xpath=ancestor::form");
      assert.equal(
        new URL((await removeForm.getAttribute("action")) ?? "", page.url()).pathname,
        "/dashboard/machines/removable-kata-bot/remove",
        "removal must use a stable POST URL instead of a build-specific Server Action id"
      );
      assert.equal((await removeForm.getAttribute("method"))?.toLowerCase(), "post");

      page.once("dialog", (dialog) => dialog.accept());
      await removeButton.click();
      await page.waitForURL(
        /\/dashboard\?new=1&agentRemoval=requested$/u
      );
      await expectVisibleText(page, "Agent removal started");
      await expectVisibleText(
        page,
        "Its compute is being removed. Saved agent data is retained. It will disappear from your dashboard when removal finishes."
      );
      assert.deepEqual(core.state.destroyPosts, ["project_removable"]);
    });
  } finally {
    await browser?.close().catch(() => {});
    dashboard.kill("SIGTERM");
    core.server.close();
    hostedDevice.close();
    brain.server.close();
    sites.server.close();
    await Promise.race([
      once(dashboard, "exit"),
      new Promise((resolve) => setTimeout(resolve, 2_000)),
    ]);
  }
});

function startDashboard(
  port: number,
  coreUrl: string,
  hostedDeviceUrl: string,
  brainUrl: string,
  sitesUrl: string
) {
  return spawn(
    process.execPath,
    ["node_modules/next/dist/bin/next", "dev", "--hostname", "127.0.0.1", "--port", String(port)],
    {
      cwd: process.cwd(),
      env: {
        ...process.env,
        FC_CORE_API_TOKEN: CORE_TOKEN,
        FC_CORE_BASE_URL: coreUrl,
        FINITECHAT_HOSTED_API_TOKEN: HOSTED_DEVICE_TOKEN,
        FC_HOSTED_WEB_DEVICE_URL: hostedDeviceUrl,
        FC_BRAIN_UPSTREAM_URL: brainUrl,
        FC_SITES_UPSTREAM_URL: sitesUrl,
        FINITE_SITES_VIEWER_SESSION_TOKEN: SITES_VIEWER_SESSION_TOKEN,
        FC_SITES_ALLOW_LOCAL_OUTPUTS: "1",
        FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
        FC_DASHBOARD_DEV_EMAIL: "browser@finite.vip",
        FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_browser",
        FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "fixture-browser-access-token",
        FC_DASHBOARD_RUNTIME_MODE: "canary",
        WORKOS_COOKIE_PASSWORD: "browser-test-cookie-password-32-characters-minimum",
        FC_WORKOS_AUTH_ENABLED: "0",
        NEXT_DIST_DIR: ".next-browser-test",
      },
      stdio: "pipe",
    }
  );
}

async function startFakeSites() {
  const state: FakeSitesState = {
    exchanges: [],
    redemptions: 0,
    privateContentRequests: 0,
  };
  let siteUrl = "";
  const token = "cd".repeat(32);
  const server = http.createServer(async (request, response) => {
    const requestUrl = new URL(request.url ?? "/", `http://${request.headers.host ?? "localhost"}`);
    if (request.method === "POST" && requestUrl.pathname === "/internal/v1/viewer-sessions") {
      const body = (await readJson(request)) as Record<string, unknown>;
      const exchange = {
        authorization: singleHeader(request.headers.authorization),
        outputUrl: String(body.output_url ?? ""),
        verifiedEmail: String(body.verified_email ?? ""),
        returnTo: String(body.return_to ?? ""),
      };
      state.exchanges.push(exchange);
      if (
        exchange.authorization !== `Bearer ${SITES_VIEWER_SESSION_TOKEN}`
        || exchange.outputUrl !== siteUrl
        || exchange.verifiedEmail !== "browser@finite.vip"
        || exchange.returnTo !== "/"
      ) {
        writeJson(response, 403, { error: "viewer access unavailable" });
        return;
      }
      writeJson(response, 200, {
        redeem_url: `${siteUrl}_finite/auth?token=${token}&return_to=%2F`,
      });
      return;
    }

    if (request.method === "GET" && requestUrl.pathname === "/_finite/auth") {
      if (requestUrl.searchParams.get("token") !== token) {
        response.writeHead(400).end();
        return;
      }
      state.redemptions += 1;
      response.writeHead(303, {
        location: requestUrl.searchParams.get("return_to") ?? "/",
        "set-cookie": [
          "finite_site_auth=browser-viewer; Path=/; Max-Age=600; HttpOnly; SameSite=None; Secure",
          "__Host-finite_site_auth_partitioned=browser-viewer; Path=/; Max-Age=600; HttpOnly; SameSite=None; Secure; Partitioned",
        ],
      });
      response.end();
      return;
    }

    if (request.method === "GET" && requestUrl.pathname === "/") {
      state.privateContentRequests += 1;
      const cookie = singleHeader(request.headers.cookie) ?? "";
      if (
        !cookie.includes("finite_site_auth=browser-viewer")
        && !cookie.includes("__Host-finite_site_auth_partitioned=browser-viewer")
      ) {
        response.writeHead(401, { "content-type": "text/html; charset=utf-8" });
        response.end("<!doctype html><h1>Sign in required</h1>");
        return;
      }
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      response.end("<!doctype html><h1>Private site browser proof</h1>");
      return;
    }
    writeJson(response, 404, { error: "not found" });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");
  siteUrl = `http://browser-proof.sites.localhost:${address.port}/`;
  return {
    server,
    state,
    apiUrl: `http://127.0.0.1:${address.port}`,
    siteUrl,
  };
}

async function startFakeBrain() {
  const server = http.createServer((request, response) => {
    if (request.method === "GET" && request.url === "/client") {
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      response.end(`<!doctype html><html><body><main><h1>FiniteBrain browser proof</h1><p>First-party client origin reached.</p><p id="api-status">Connecting…</p></main><script>fetch('/_admin/browser-proof').then((result) => result.json()).then((result) => { document.getElementById('api-status').textContent = result.message; });</script></body></html>`);
      return;
    }
    if (request.method === "GET" && request.url === "/_admin/browser-proof") {
      writeJson(response, 200, { message: "Brain API ready" });
      return;
    }
    writeJson(response, 404, { error: "not found" });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");
  return { server, url: `http://127.0.0.1:${address.port}` };
}

async function startFakeHostedDevice() {
  const state: HostedDeviceState = {
    app: initialHostedChatState(),
    actions: [],
    runtimeCommands: [],
    authRequests: [],
    connections: {
      inference: {
        profile: "finite_private",
        provider: "finite_private",
        model: "openai/gpt-oss-120b",
      },
      telegram: {
        connected: false,
        home_channel: null,
        pending: [],
        approved: [],
      },
      google: {
        connected: false,
        email: null,
      },
    },
  };
  const streams = new Set<ServerResponse>();
  const server = http.createServer(async (request, response) => {
    try {
      await handleHostedDeviceRequest(request, response, state, streams);
    } catch (error) {
      if (!response.headersSent) {
        response.writeHead(500, { "content-type": "application/json" });
      }
      response.end(JSON.stringify({ error: String(error) }));
    }
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");
  const url = `http://127.0.0.1:${address.port}`;

  return {
    server,
    state,
    url,
    runtimeStatusUrl: `${url}/runtime-status`,
    emit() {
      state.app.rev += 1;
      emitHostedState(streams, state.app);
    },
    close() {
      for (const stream of streams) {
        stream.end();
      }
      server.close();
    },
  };
}

async function handleHostedDeviceRequest(
  request: IncomingMessage,
  response: ServerResponse,
  state: HostedDeviceState,
  streams: Set<ServerResponse>
) {
  const path = request.url ?? "/";
  if (request.method === "GET" && path === "/runtime-status") {
    writeJson(response, 200, {
      paired: true,
      agent_npub: AGENT_NPUB,
      room_id: "room_browser_agent",
    });
    return;
  }

  if (!path.startsWith("/v1/app/")) {
    writeJson(response, 404, { error: "not found" });
    return;
  }

  const authRequest: HostedAuthRequest = {
    method: request.method ?? "GET",
    path,
    authorization: singleHeader(request.headers.authorization),
    workosUserId: singleHeader(request.headers["x-finite-workos-user-id"]),
  };
  state.authRequests.push(authRequest);
  if (
    authRequest.authorization !== `Bearer ${HOSTED_DEVICE_TOKEN}`
    || authRequest.workosUserId !== "user_browser"
  ) {
    writeJson(response, 401, { error: "missing hosted-device credentials" });
    return;
  }

  if (request.method === "GET" && path.startsWith("/v1/app/attachments/")) {
    response.writeHead(200, {
      "cache-control": "private, no-store",
      "content-disposition": "inline; filename=browser-proof.png",
      "content-length": String(PNG_BYTES.length),
      "content-type": "image/png",
    });
    response.end(PNG_BYTES);
    return;
  }

  if (request.method === "POST" && path === "/v1/app/images") {
    await readBytes(request);
    writeJson(response, 200, { image_url: AGENT_PICTURE_URL });
    return;
  }

  if (request.method === "POST" && path === "/v1/app/runtime-commands") {
    const command = (await readJson(request)) as Record<string, unknown>;
    state.runtimeCommands.push(command);
    writeJson(response, 200, applyRuntimeCommand(state, command));
    return;
  }

  if (request.method === "POST" && path === "/v1/app/attachments") {
    await readBytes(request);
    state.app.messages.push(
      hostedImageMessage("Image sent from browser.", true, state.app.messages.length + 1, "browser-proof.png")
    );
    state.app.rev += 1;
    emitHostedState(streams, state.app);
    writeJson(response, 200, state.app);
    return;
  }

  if (request.method === "GET" && path === "/v1/app/state") {
    writeJson(response, 200, state.app);
    return;
  }

  if (request.method === "POST" && path === "/v1/app/actions") {
    const action = (await readJson(request)) as Record<string, unknown>;
    state.actions.push(action);
    applyHostedAction(state.app, action);
    emitHostedState(streams, state.app);
    writeJson(response, 200, state.app);
    return;
  }

  if (request.method === "GET" && path === "/v1/app/updates") {
    response.writeHead(200, {
      "cache-control": "no-cache",
      connection: "keep-alive",
      "content-type": "text/event-stream",
    });
    streams.add(response);
    response.on("close", () => streams.delete(response));
    writeHostedState(response, state.app);
    return;
  }

  writeJson(response, 404, { error: "not found" });
}

function applyRuntimeCommand(
  state: HostedDeviceState,
  request: Record<string, unknown>
) {
  const command = String(request.command ?? "");
  const body = request.body && typeof request.body === "object"
    ? request.body as Record<string, unknown>
    : {};
  if (command === "agent.inference.apply") {
    const profile = String(body.profile ?? "");
    if (profile === "finite_private") {
      state.connections.inference = {
        profile,
        provider: "finite_private",
        model: "openai/gpt-oss-120b",
      };
    } else if (profile === "openrouter") {
      state.connections.inference = {
        profile,
        provider: "openrouter",
        model: String(body.model ?? "anthropic/claude-sonnet-4.6"),
      };
    }
  } else if (command === "agent.telegram.connect") {
    state.connections.telegram.connected = true;
  } else if (command === "agent.telegram.disconnect") {
    state.connections.telegram.connected = false;
    state.connections.telegram.home_channel = null;
  } else if (command === "agent.google.disconnect") {
    state.connections.google.connected = false;
    state.connections.google.email = null;
  }
  return {
    request_id: `browser-command-${state.runtimeCommands.length}`,
    status: "succeeded",
    body: command === "agent.connections.status" ? state.connections : {},
    error: null,
  };
}

function initialHostedChatState(): FakeHostedChatState {
  return {
    rev: 1,
    identity: {
      account_id: "browser-user-account",
      device_id: "hosted-web",
    },
    rooms: [],
    selected_room_id: null,
    topics: [],
    selected_topic_id: null,
    selected_chat_id: null,
    active_profile_id: null,
    status: "Stopped",
    toast: null,
    messages: [],
    profiles: [],
    typing_members: [],
    flow: {
      notice_text: null,
      notice_busy: false,
      scan_in_flight: false,
      scan_result: "",
    },
  };
}

function applyHostedAction(
  state: FakeHostedChatState,
  action: Record<string, unknown>
) {
  const operation = actionName(action);
  if (operation === "StartRuntime") {
    state.status = "Runtime running";
  } else if (operation === "ScanTarget") {
    state.active_profile_id = "agent-account-browser";
    state.profiles = [
      {
        account_id: "agent-account-browser",
        npub: AGENT_NPUB,
        display_name: "Completed Oslo Bot",
        about: "Browser-test agent",
        picture: null,
        stale: false,
        is_agent: true,
      },
    ];
  } else if (operation === "StartProfileChat") {
    state.rooms = [
      {
        room_id: "room_browser_agent",
        display_name: "Completed Oslo Bot",
        state: "Connected",
        status: "Connected",
        user_status_text: "Connected",
        last_message_preview: "Hello from Completed Oslo Bot.",
        unread_count: 0,
        is_agent_chat: true,
      },
    ];
    state.selected_room_id = "room_browser_agent";
    state.topics = [
      {
        room_id: "room_browser_agent",
        topic_id: "topic_browser_agent",
        title: "General",
        active_chat_id: "chat_browser_agent",
        chats: [
          {
            chat_id: "chat_browser_agent",
            title: "General",
            active: true,
          },
        ],
      },
    ];
    state.selected_topic_id = "topic_browser_agent";
    state.selected_chat_id = "chat_browser_agent";
    state.messages = [hostedMessage("Hello from Completed Oslo Bot.", false, 1)];
  } else if (operation === "SendChatMessage") {
    const payload = action.SendChatMessage as Record<string, unknown> | undefined;
    assert(payload);
    const text = String(payload.text ?? "");
    assert(text);
    state.messages.push(hostedMessage(text, true, state.messages.length + 1));
    state.rooms[0]!.last_message_preview = text;
  } else if (operation === "RenameChat") {
    const payload = action.RenameChat as Record<string, unknown> | undefined;
    assert(payload);
    const title = String(payload.title ?? "");
    assert(title);
    state.topics[0]!.chats[0]!.title = title;
  }
  state.rev += 1;
}

function hostedMessage(
  text: string,
  isMine: boolean,
  seq: number
): FakeHostedChatState["messages"][number] {
  return {
    room_id: "room_browser_agent",
    seq,
    message_id: `message_${seq}`,
    conversation_id: "topic_browser_agent",
    chat_id: "chat_browser_agent",
    sender_account_id: isMine ? "browser-user-account" : "agent-account-browser",
    sender_display_name: isMine ? "You" : "Completed Oslo Bot",
    text,
    display_content: text,
    kind: "message",
    status: "complete",
    final_delivery: false,
    edit_of_message_id: null,
    is_mine: isMine,
    media: [],
    timestamp_unix_seconds: 1_780_000_000 + seq,
    display_timestamp: "12:00 PM",
  };
}

function hostedImageMessage(
  text: string,
  isMine: boolean,
  seq: number,
  filename: string
): FakeHostedChatState["messages"][number] {
  return {
    ...hostedMessage(text, isMine, seq),
    kind: "media",
    media: [
      {
        attachment_id: `attachment_${seq}`,
        mime_type: "image/png",
        filename,
        kind: "Image",
        width: 1,
        height: 1,
      },
    ],
  };
}

function emitHostedState(
  streams: Set<ServerResponse>,
  state: FakeHostedChatState
) {
  for (const stream of streams) {
    writeHostedState(stream, state);
  }
}

function writeHostedState(response: ServerResponse, state: FakeHostedChatState) {
  response.write(`id: ${state.rev}\nevent: state\ndata: ${JSON.stringify(state)}\n\n`);
}

function actionName(action: Record<string, unknown>) {
  return Object.keys(action)[0] ?? "";
}

function singleHeader(value: string | string[] | undefined) {
  return Array.isArray(value) ? (value[0] ?? null) : (value ?? null);
}

async function startFakeCore() {
  let state = emptyCoreState();
  const server = http.createServer(async (request, response) => {
    try {
      await handleCoreRequest(request, response, state);
    } catch (error) {
      response.writeHead(500, { "content-type": "application/json" });
      response.end(JSON.stringify({ error: String(error) }));
    }
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");

  return {
    server,
    url: `http://127.0.0.1:${address.port}`,
    get state() {
      return state;
    },
    reset(next: Partial<CoreState> = {}) {
      state = { ...emptyCoreState(), ...next };
    },
  };
}

async function handleCoreRequest(
  request: IncomingMessage,
  response: ServerResponse,
  state: CoreState
) {
  if (
    request.headers.authorization !== `Bearer ${CORE_TOKEN}` &&
    request.headers.authorization !== "Bearer fixture-browser-access-token"
  ) {
    writeJson(response, 401, { error: "missing service token" });
    return;
  }

  if (request.method === "GET" && request.url === "/api/core/v1/me") {
    writeJson(response, 200, {
      email: "browser@finite.vip",
      workos_user_id: "user_browser",
      claimable_candidates: [],
      projects: state.projects,
      agent_creation_requests: state.requests,
    });
    return;
  }

  if (request.method === "GET" && request.url === "/api/core/v1/me/billing") {
    writeJson(response, 200, {
      customer_org: {
        id: "org_browser",
        owner_user_id: "user_browser",
        name: "Browser Test",
        billing_class: "sponsored",
        created_at: "2026-05-28T12:00:00Z",
        updated_at: "2026-05-28T12:01:00Z",
      },
      billing_account: null,
      agent_creation_entitlement: {
        id: "entitlement_browser",
        customer_org_id: "org_browser",
        allowed_new_agent_runtimes: 0,
        launch_code: "fixture-launch-code",
        created_at: "2026-05-28T12:00:00Z",
        updated_at: "2026-05-28T12:01:00Z",
      },
      can_create_agent: state.canCreateAgent,
      requires_billing: true,
    });
    return;
  }

  if (request.method === "POST" && request.url === "/api/core/v1/me/agent-creation-requests") {
    const body = await readJson(request);
    state.creationPosts.push(body);
    if (state.creationError) {
      writeJson(response, 402, { error: state.creationError });
      return;
    }
    if (state.createDelayMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, state.createDelayMs));
    }
    const projectId = `project_${state.creationPosts.length}`;
    const requestRecord = agentCreationRequest({
      id: `agent_request_${state.creationPosts.length}`,
      projectId,
      displayName: String(body.displayName ?? "Oslo Bot"),
      status: "requested",
      createdAt: new Date().toISOString(),
    });
    state.requests = [requestRecord];
    writeJson(response, 200, {
      reused: false,
      project: {
        id: projectId,
        customer_org_id: "org_browser",
        owner_user_id: "user_browser",
        display_name: requestRecord.display_name,
        import_candidate_id: null,
        created_at: requestRecord.created_at,
        updated_at: requestRecord.updated_at,
      },
      request: {
        id: requestRecord.id,
        customer_org_id: "org_browser",
        owner_user_id: "user_browser",
        project_id: requestRecord.project_id,
        idempotency_key: String(body.idempotencyKey ?? ""),
        display_name: requestRecord.display_name,
        runner_class: String(body.runnerClass ?? "apple_container"),
        profile_picture_url: body.profilePictureUrl ?? null,
        status: requestRecord.status,
        requested_launch_code: String(body.launchCode ?? ""),
        agent_runtime_id: null,
        created_at: requestRecord.created_at,
        updated_at: requestRecord.updated_at,
      },
    });
    return;
  }

  const destroyMatch = request.url?.match(
    /^\/api\/core\/v1\/me\/projects\/([^/]+)\/runtime\/destroy$/u
  );
  if (request.method === "POST" && destroyMatch?.[1]) {
    const projectId = decodeURIComponent(destroyMatch[1]);
    const project = state.projects.find(
      (candidate) => candidate.project.id === projectId
    );
    state.destroyPosts.push(projectId);
    writeJson(response, 200, {
      id: `runtime_control_destroy_${state.destroyPosts.length}`,
      project_id: projectId,
      agent_runtime_id: project?.runtime?.id ?? "runtime_missing",
      source_host_id: project?.runtime?.source_host_id ?? "oslo",
      source_machine_id: project?.runtime?.source_machine_id ?? "missing",
      requested_by_user_id: "user_browser",
      kind: "destroy",
      status: "requested",
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    });
    return;
  }

  const cancelMatch = request.url?.match(/^\/api\/core\/v1\/agent-creation-requests\/([^/]+)\/cancel$/u);
  if (request.method === "POST" && cancelMatch?.[1]) {
    const requestId = decodeURIComponent(cancelMatch[1]);
    state.cancelPosts.push(requestId);
    state.requests = state.requests.filter((candidate) => candidate.id !== requestId);
    writeJson(response, 200, agentCreationRequest({ id: requestId, projectId: "cancelled", status: "cancelled" }));
    return;
  }

  writeJson(response, 404, { error: "not found" });
}

async function withSignedInPage(
  browser: Browser,
  dashboardPort: number,
  fn: (page: Page) => Promise<void>
) {
  const context = await browser.newContext();
  try {
    const page = await context.newPage();
    await fn(page);
  } finally {
    await context.close();
  }
}

function emptyCoreState(): CoreState {
  return {
    projects: [],
    requests: [],
    creationPosts: [],
    cancelPosts: [],
    destroyPosts: [],
    createDelayMs: 0,
    canCreateAgent: false,
    creationError: null,
  };
}

function agentCreationRequest({
  id,
  projectId,
  displayName = "Oslo Bot",
  status,
  failureMessage = null,
  createdAt = new Date().toISOString(),
  runnerClass = "apple_container",
  agentRuntimeId = null,
}: {
  id: string;
  projectId: string;
  displayName?: string;
  status: AgentCreationRequest["status"];
  failureMessage?: string | null;
  createdAt?: string;
  runnerClass?: AgentCreationRequest["runner_class"];
  agentRuntimeId?: string | null;
}): AgentCreationRequest {
  return {
    id,
    project_id: projectId,
    display_name: displayName,
    runner_class: runnerClass,
    profile_picture_url: null,
    status,
    agent_runtime_id: agentRuntimeId,
    failure_message: failureMessage,
    created_at: createdAt,
    updated_at: createdAt,
  };
}

function visibleProject(
  projectId: string,
  displayName: string,
  runtimeStatusUrl: string,
  machineId = "completed-oslo-bot"
): VisibleProject {
  return {
    project: {
      id: projectId,
      customer_org_id: "org_browser",
      owner_user_id: "user_browser",
      display_name: displayName,
      import_candidate_id: null,
      created_at: "2026-05-28T12:00:00Z",
      updated_at: "2026-05-28T12:01:00Z",
    },
    runtime: {
      id: `runtime_${machineId}`,
      project_id: projectId,
      source_host_id: "oslo",
      source_machine_id: machineId,
      source_import_key: `oslo:${machineId}`,
      runtime_artifact_id: "runtime_artifact_1",
      state_schema_version: "runtime-state-v1",
      host_facts: {
        display_name: "Completed Oslo Bot",
        hostname: `${machineId}.finite.computer`,
        runtime_host: "oslo",
        runtime_status: "online",
        hermes_available: true,
        published_app_urls: [runtimeStatusUrl],
      },
      created_at: "2026-05-28T12:00:00Z",
      updated_at: "2026-05-28T12:01:00Z",
    },
  };
}

async function readJson(request: IncomingMessage) {
  return JSON.parse((await readBytes(request)).toString("utf8"));
}

async function readBytes(request: IncomingMessage) {
  const chunks: Buffer[] = [];
  for await (const chunk of request) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  }
  return Buffer.concat(chunks);
}

function writeJson(response: ServerResponse, status: number, body: unknown) {
  response.writeHead(status, { "content-type": "application/json" });
  response.end(JSON.stringify(body));
}

async function waitForDashboard(port: number, output: () => string) {
  await waitFor(async () => {
    const response = await fetch(`http://127.0.0.1:${port}/`, { redirect: "manual" }).catch(() => null);
    return Boolean(response && response.status < 500);
  }, 30_000, () => `dashboard did not become ready\n${output()}`);
}

async function expectVisibleText(page: Page, text: string) {
  await page.getByText(text, { exact: true }).waitFor({ state: "visible", timeout: 15_000 });
}

async function waitFor(
  condition: () => boolean | Promise<boolean>,
  timeoutMs = 5_000,
  timeoutMessage: () => string | Promise<string> = () => "timed out waiting for condition"
) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    if (await condition()) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(await timeoutMessage());
}

async function pageText(page: Page) {
  return (await page.locator("body").innerText({ timeout: 1_000 }).catch((error) => String(error))).slice(0, 4_000);
}

function collectOutput(process: ChildProcessWithoutNullStreams) {
  let output = "";
  process.stdout.on("data", (chunk) => {
    output = `${output}${chunk.toString()}`.slice(-8_000);
  });
  process.stderr.on("data", (chunk) => {
    output = `${output}${chunk.toString()}`.slice(-8_000);
  });
  return () => output;
}

async function freePort() {
  const server = http.createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");
  const { port } = address;
  server.close();
  await once(server, "close");
  return port;
}

function chromeExecutable() {
  for (const executablePath of [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
  ]) {
    if (existsSync(executablePath)) {
      return { executablePath };
    }
  }
  return { channel: "chrome" as const };
}
