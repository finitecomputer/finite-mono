import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import http, { type IncomingMessage, type ServerResponse } from "node:http";
import { once } from "node:events";
import { test } from "node:test";

import { chromium, type Browser, type Page } from "playwright";

import { chromiumLaunchOptions } from "../scripts/playwright-browser";

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
    display_name: string;
    created_at: string;
    updated_at: string;
  };
  runtime: null | {
    id: string;
    project_id: string;
    contact_endpoint: string;
    runtime_status: "online" | "offline" | "stale" | "unknown";
    hermes_available: boolean;
    runtime_capabilities?: {
      restart?: boolean;
      recover_known_good_chat?: boolean;
      runtime_upgrade?: boolean;
      stop?: boolean;
      runtime_retirement?: boolean;
    } | null;
    created_at: string;
    updated_at: string;
  };
};

type CoreState = {
  projects: VisibleProject[];
  requests: AgentCreationRequest[];
  creationPosts: unknown[];
  creationResults: Map<string, { projectId: string; requestId: string }>;
  meGets: number;
  runtimeRouteGets: number;
  runtimeRouteProjectIdOverride: string | null;
  cancelPosts: string[];
  destroyPosts: string[];
  recoverPosts: string[];
  restartPosts: string[];
  createDelayMs: number;
  canCreateAgent: boolean;
  requiresBilling: boolean;
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
      kind: "Image" | "VoiceNote" | "Video" | "File";
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
  hosted_agent_binding: {
    version: number;
    project_id: string;
    human_account_id: string;
    agent_account_id: string;
    agent_npub: string;
    canonical_room_id: string;
    associated_room_ids: string[];
  } | null;
  profiles: Array<{
    account_id: string;
    npub: string;
    display_name: string;
    about: string | null;
    picture: string | null;
    stale: boolean;
    is_agent: boolean;
  }>;
  devices: Array<{
    account_id: string;
    device_id: string;
    active: boolean;
    current_device: boolean;
    revoked: boolean;
    room_count: number;
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
  unavailable: boolean;
  ownerClaimGate: Promise<void> | null;
  releaseOwnerClaimGate: (() => void) | null;
  navigationActionGate: Promise<void> | null;
  releaseNavigationActionGate: (() => void) | null;
  completedSelectionMutations: number;
  app: FakeHostedChatState;
  actions: Array<Record<string, unknown>>;
  newChatRequests: Array<Record<string, unknown>>;
  runtimeCommands: Array<Record<string, unknown>>;
  authRequests: HostedAuthRequest[];
  bindingAuthorizations: Array<{
    project_id: string;
    creation_request_id: string;
  }>;
  bindingAuthorizationFailuresRemaining: number;
  agentBindings: Map<
    string,
    NonNullable<FakeHostedChatState["hosted_agent_binding"]>
  >;
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

test("dashboard agent creation browser states", { timeout: 180_000 }, async () => {
  const hostedDevice = await startFakeHostedDevice();
  const core = await startFakeCore(() => hostedDevice.setAvailable(true));
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
  const paidDashboardPort = await freePort();
  const paidDashboard = startDashboard(
    paidDashboardPort,
    core.url,
    hostedDevice.url,
    brain.url,
    sites.apiUrl,
    { stripeConfigured: true, distDir: ".next-browser-stripe-test" }
  );
  const paidDashboardOutput = collectOutput(paidDashboard);
  let browser: Browser | null = null;

  try {
    await waitForDashboard(dashboardPort, dashboardOutput);
    await waitForDashboard(paidDashboardPort, paidDashboardOutput);
    browser = await chromium.launch({
      headless: true,
      ...chromiumLaunchOptions(),
    });

    core.reset({
      canCreateAgent: true,
      requiresBilling: false,
    });
    await withSignedInPage(browser, paidDashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${paidDashboardPort}/dashboard?new=1`);
      await page.getByLabel("Agent name").fill("Customer Access Proof");
      await page.getByRole("button", { name: "Continue" }).waitFor({ state: "visible" });
      assert.equal(
        await page.getByRole("button", { name: "Create agent" }).count(),
        0,
        "fresh customer onboarding must never bypass Access from Profile"
      );
      await page.getByRole("button", { name: "Continue" }).click();
      await page
        .getByRole("button", { name: "Continue to secure payment" })
        .waitFor({ state: "visible" });
      await expectVisibleText(page, "Pay securely or use a Launch Code.");
      await expectVisibleText(page, "Finite Computer Hosted Agent");
      await expectVisibleText(page, "$200 USD / month");
      await page
        .getByText("Renews monthly until you cancel in the billing portal.")
        .waitFor({ state: "visible", timeout: 15_000 });
      await page
        .getByText("refunds are handled per our")
        .waitFor({ state: "visible", timeout: 15_000 });
      assert.equal(
        core.state.creationPosts.length,
        0,
        "Profile must not submit agent creation before the customer chooses Access"
      );
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
      await page.getByRole("button", { name: "Account menu" }).waitFor({ state: "visible" });
      await agentName.fill("Oslo Bot");
      await page.locator('#coreAgentPicture').setInputFiles({
        name: "oslo-bot.png",
        mimeType: "image/png",
        buffer: PNG_BYTES,
      });
      await page.getByRole("img", { name: "Agent profile preview" }).waitFor({ state: "visible" });
      await page.getByRole("button", { name: "Continue" }).click();
      assert.equal(
        await page.getByRole("button", { name: "Continue to secure payment" }).count(),
        0,
        "payment must stay hidden when the webhook/return path is not fully configured"
      );
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
      assert.equal("runnerClass" in post, false);
      assert.equal(post.profilePictureUrl, AGENT_PICTURE_URL);
      assert.match(String(post.idempotencyKey), /.+/);
      await page.waitForURL(/\/dashboard\?new=1&creation=agent_request_1$/u);
      assert.deepEqual(hostedDevice.state.bindingAuthorizations.at(-1), {
        project_id: "project_1",
        creation_request_id: "agent_request_1",
      });
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

    core.reset();
    hostedDevice.failNextBindingAuthorization();
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await page.getByLabel("Agent name").fill("Authorization Retry Bot");
      await page.getByRole("button", { name: "Continue" }).click();
      await page.getByLabel("Launch Code").fill("fixture-launch-code");
      await page.getByRole("button", { name: "Apply" }).click();
      await page.waitForURL((url) => url.searchParams.has("agentCreationError"));
      assert.equal(
        new URL(page.url()).searchParams.get("agentCreationError"),
        "binding authorization is temporarily unavailable"
      );
      assert(
        (await page.context().cookies()).some(
          (cookie) => cookie.name === "finite-agent-draft"
        ),
        "binding authorization failure did not preserve the signed draft cookie"
      );
      const retryName = page.getByLabel("Agent name");
      await retryName.waitFor({ state: "visible" }).catch(async (error) => {
        throw new Error(
          `binding authorization retry form did not render: ${String(error)}\n${await pageText(page)}`
        );
      });
      await expectVisibleText(page, "binding authorization is temporarily unavailable");
      await retryName.fill("Authorization Retry Bot");
      await page.getByRole("button", { name: "Continue" }).click();
      await page.getByLabel("Launch Code").fill("fixture-launch-code");
      await page.getByRole("button", { name: "Apply" }).click();
      await page.waitForURL(/\/dashboard\?new=1&creation=agent_request_1$/u);

      assert.equal(core.state.creationPosts.length, 2);
      assert.equal(
        (core.state.creationPosts[0] as Record<string, unknown>).idempotencyKey,
        (core.state.creationPosts[1] as Record<string, unknown>).idempotencyKey,
        "authorization retry did not reuse the successful Core creation request"
      );
      assert.deepEqual(hostedDevice.state.bindingAuthorizations.slice(-2), [
        {
          project_id: "project_1",
          creation_request_id: "agent_request_1",
        },
        {
          project_id: "project_1",
          creation_request_id: "agent_request_1",
        },
      ]);
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
      await expectVisibleText(page, "Choose payment or enter a Launch Code to continue.");
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
      const completedAgentHeading = page.getByRole("heading", {
        name: "Completed Oslo Bot",
        exact: true,
      });
      await waitFor(
        async () => {
          if (await completedAgentHeading.isVisible()) return true;
          await page.waitForTimeout(250);
          await page.reload();
          return completedAgentHeading.isVisible();
        },
        15_000,
        async () => `account agent cards did not hydrate from Core\n${await pageText(page)}`
      );
      await page.getByRole("heading", { name: "Your agents" }).waitFor({ state: "visible" });
      await completedAgentHeading
        .locator("xpath=ancestor::section[1]")
        .getByRole("link", { name: "Agent", exact: true })
        .click();
      await page.waitForURL(/\/dashboard\/machines\/runtime_completed-oslo-bot$/u);
      const main = page.getByRole("main");
      await expectVisibleText(page, "Your agent is online.");
      const productNav = page.getByRole("navigation", { name: "Agent navigation" });
      const agentLink = productNav.getByRole("link", { name: "Agent", exact: true });
      // The fake Core changes out of band, unlike a product mutation that
      // invalidates the dashboard's short SWR cache. A stale server render
      // starts the background refresh; reload until that refreshed projection
      // is visible instead of racing one fixed sleep against CI load.
      await waitFor(
        async () => {
          if (
            await agentLink.isVisible()
            && (await agentLink.getAttribute("aria-current")) === "page"
          ) {
            return true;
          }
          await page.waitForTimeout(250);
          await page.reload();
          return (
            await agentLink.isVisible()
            && (await agentLink.getAttribute("aria-current")) === "page"
          );
        },
        15_000,
        async () => `agent navigation did not hydrate from Core\nURL: ${page.url()}\n${await pageText(page)}\n${dashboardOutput()}`
      );
      await productNav.getByRole("link", { name: "Connections", exact: true }).waitFor({ state: "visible" });
      await productNav.getByRole("link", { name: "Brain", exact: true }).waitFor({ state: "visible" });
      assert.equal(await productNav.getByRole("link", { name: "Skills", exact: true }).count(), 0);
      await page
        .getByRole("navigation", { name: "Agent, topics, and chats" })
        .waitFor({ state: "visible" });

      const machineSwitcher = page.getByRole("button", {
        name: "Completed Oslo Bot",
        exact: true,
      });
      await machineSwitcher.waitFor({ state: "visible" });
      await machineSwitcher.click();
      const secondAgentItem = page.getByRole("menuitem", {
        name: "Second Oslo Bot",
        exact: true,
      });
      await secondAgentItem.waitFor({ state: "visible" });
      await secondAgentItem.click();
      await page.waitForURL(/\/dashboard\/machines\/runtime_second-oslo-bot$/u);

      const secondMachineSwitcher = page.getByRole("button", {
        name: "Second Oslo Bot",
        exact: true,
      });
      await secondMachineSwitcher.waitFor({ state: "visible" });
      await secondMachineSwitcher.click();
      const newAgentItem = page.getByRole("menuitem", { name: "New agent", exact: true });
      await newAgentItem.waitFor({ state: "visible" });
      await newAgentItem.click();
      await page.waitForURL(
        /\/dashboard\?new=1&machine=runtime_second-oslo-bot$/u
      );
      await page.getByLabel("Agent name").waitFor({ state: "visible" });
      const returnToExistingChat = page.getByRole("link", {
        name: "Return to Second Oslo Bot chat",
        exact: true,
      });
      await returnToExistingChat.waitFor({ state: "visible" });
      assert.equal(
        await returnToExistingChat.getAttribute("href"),
        "/dashboard/machines/runtime_second-oslo-bot/chat"
      );
      await returnToExistingChat.click();
      await page.waitForURL(
        /\/dashboard\/machines\/runtime_second-oslo-bot\/chat$/u
      );
      const existingComposer = page.getByLabel("Message your agent");
      await existingComposer.waitFor({ state: "visible" });
      await waitFor(
        async () => existingComposer.isEnabled(),
        5_000,
        () => "returning from New agent did not restore the existing chat composer"
      );

      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/runtime_second-oslo-bot`
      );
      await secondMachineSwitcher.waitFor({ state: "visible" });
      await secondMachineSwitcher.click();
      await page.getByRole("menuitem", { name: "New agent", exact: true }).click();
      await page.waitForURL(
        /\/dashboard\?new=1&machine=runtime_second-oslo-bot$/u
      );
      await page.getByLabel("Agent name").waitFor({ state: "visible" });
      await page.getByLabel("Agent name").fill("Second Oslo Bot");
      await page.getByRole("button", { name: "Continue" }).click();
      await page.getByLabel("Launch Code").waitFor({ state: "visible" });
      await page.getByRole("button", { name: "Apply", exact: true }).click();
      await page.waitForURL((url) => {
        const params = url.searchParams;
        return (
          url.pathname === "/dashboard" &&
          params.get("new") === "1" &&
          params.get("machine") === "runtime_second-oslo-bot" &&
          Boolean(params.get("agentCreationError"))
        );
      });
      await page
        .getByRole("paragraph")
        .filter({ hasText: /^Enter your Launch Code\.$/u })
        .waitFor({ state: "visible" });
      assert.equal(
        await page
          .getByRole("link", {
            name: "Return to Second Oslo Bot chat",
            exact: true,
          })
          .getAttribute("href"),
        "/dashboard/machines/runtime_second-oslo-bot/chat",
        "a server redirect lost the originating agent"
      );

      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard?new=1&creation=agent_request_second`
      );
      await page.waitForURL(/\/dashboard\/machines\/runtime_second-oslo-bot$/u);

      hostedDevice.holdOwnerClaim();
      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/completed-oslo-bot`
      );
      await page.waitForURL(/\/dashboard\/machines\/runtime_completed-oslo-bot$/u);
      await main.getByRole("button", { name: "Restart agent" }).waitFor({ state: "visible" });
      assert.equal(
        await main.getByRole("button", { name: "Recover chat" }).count(),
        0,
        "recovery must stay hidden when Core explicitly advertises it as unsupported"
      );
      assert.equal(await page.getByText("Chat recovery", { exact: true }).count(), 0);
      await main.getByRole("button", { name: "Stop" }).waitFor({ state: "visible" });
      assert.equal(await main.getByRole("button", { name: "Destroy" }).count(), 0);
      const openWebChat = main.getByRole("link", { name: "Open chat" });
      await openWebChat.waitFor({ state: "visible" });

      const bindingAuthorizationCount =
        hostedDevice.state.bindingAuthorizations.length;
      core.state.runtimeRouteProjectIdOverride = "project_second";
      await openWebChat.click();
      await page.waitForURL(/\/dashboard\/machines\/runtime_completed-oslo-bot\/chat$/u);
      await page
        .getByRole("button", { name: "Finish chat setup", exact: true })
        .first()
        .waitFor({ state: "visible" });
      assert.equal(
        hostedDevice.state.bindingAuthorizations.length,
        bindingAuthorizationCount,
        "ordinary chat load granted binding bootstrap authority"
      );
      assert.equal(core.state.creationPosts.length, 0, "chat recovery created another Project");
      const recoveryMeGets = core.state.meGets;
      const recoveryRuntimeRouteGets = core.state.runtimeRouteGets;
      await page
        .getByRole("button", { name: "Finish chat setup", exact: true })
        .first()
        .click();
      await waitFor(
        () => hostedDevice.state.bindingAuthorizations.length === bindingAuthorizationCount + 1
      );
      assert.equal(
        core.state.meGets,
        recoveryMeGets + 1,
        "binding recovery did not use exactly one fresh Core snapshot"
      );
      assert.equal(
        core.state.runtimeRouteGets,
        recoveryRuntimeRouteGets,
        "binding recovery consulted a conflicting runtime-route snapshot"
      );
      core.state.runtimeRouteProjectIdOverride = null;
      assert.deepEqual(hostedDevice.state.bindingAuthorizations.at(-1), {
        project_id: "project_running",
        creation_request_id: "agent_request_old",
      });
      await expectVisibleText(page, "Hello from Completed Oslo Bot.");
      await expectVisibleText(page, "Topics");
      await page
        .getByRole("button", { name: "New chat in General", exact: true })
        .waitFor({ state: "visible" });
      await page
        .getByRole("button", { name: "New chat", exact: true })
        .waitFor({ state: "visible" });
      await expectVisibleText(page, "browser@finite.vip");
      await waitFor(
        () =>
          hostedDevice.state.runtimeCommands.some(
            (command) => command.command === "agent.owner.claim"
          ),
        5_000,
        () => "dashboard did not request the owner claim"
      );
      const composer = page.getByLabel("Message your agent");
      assert.equal(
        await composer.isDisabled(),
        true,
        "chat composer became usable before the owner claim succeeded"
      );
      const connectionsLink = page.getByRole("link", { name: "Connections", exact: true });
      await connectionsLink.first().waitFor({ state: "visible" });
      assert.match(
        (await connectionsLink.first().getAttribute("href")) ?? "",
        /\/connections$/u,
        "the shared sidebar should remain inspectable while the owner claim is pending"
      );
      hostedDevice.releaseOwnerClaim();
      await waitFor(
        async () => !(await composer.isDisabled()),
        5_000,
        () => "chat composer did not become usable after the owner claim succeeded"
      );
      await connectionsLink.first().waitFor({ state: "visible" });
      assert.equal(await page.getByRole("link", { name: "Finite.Computer" }).count(), 1);
      await connectionsLink.first().click();
      await page.waitForURL(/\/dashboard\/machines\/runtime_completed-oslo-bot\/connections$/u);
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
      await page
        .getByRole("link", { name: "Brain", exact: true })
        .waitFor({ state: "visible" });
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
        .locator(".finite-chat__topbar")
        .getByText("Browser QA", { exact: true })
        .waitFor({ state: "visible" });

      assert.equal(
        await page.getByRole("button", { name: "Devices", exact: true }).count(),
        0,
        "unsupported Devices navigation must stay out of the shared agent shell"
      );

      assert(
        hostedDevice.state.authRequests.some(
          (request) => request.path === "/v1/app/agent-bindings/open"
        ),
        "chat bootstrap did not first check for an existing canonical Agent binding"
      );
      assert(
        hostedDevice.state.authRequests.some(
          (request) => request.path === "/v1/app/agent-bindings/ensure"
        ),
        "chat bootstrap did not consume the prior creation authorization"
      );
      assert.equal(
        hostedDevice.state.bindingAuthorizations.length,
        bindingAuthorizationCount + 1,
        "only the explicit recovery action may grant Room-creation authority"
      );

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

      hostedDevice.setAvailable(false);
      await page.reload();
      await page
        .getByText("Chat needs attention", { exact: true })
        .waitFor({ state: "visible", timeout: 25_000 });
      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/completed-oslo-bot`
      );
      const restartAgent = main.getByRole("button", { name: "Restart agent" });
      await restartAgent.click();
      await waitFor(() => core.state.restartPosts.includes("project_running"));
      await restartAgent.waitFor({ state: "visible" });
      assert.equal(hostedDevice.state.unavailable, true, "restart must not fake chat recovery");
      assert.equal(await main.getByRole("button", { name: "Recover chat" }).count(), 0);
      assert.deepEqual(core.state.recoverPosts, []);
      hostedDevice.setAvailable(true);
      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/completed-oslo-bot/chat`
      );
      await page
        .getByRole("paragraph")
        .filter({ hasText: message })
        .waitFor({ state: "visible", timeout: 15_000 });

      await page.locator('input[type="file"]').setInputFiles({
        name: "browser-proof.png",
        mimeType: "image/png",
        buffer: PNG_BYTES,
      });
      await page.getByLabel("Message your agent").fill("Image from browser.");
      await page.getByRole("button", { name: "Send message" }).click();
      await page.getByRole("img", { name: "browser-proof.png" }).waitFor({ state: "visible" });

      const agentAttachmentResponse = page.waitForResponse((response) =>
        response.url().includes(
          "/hosted-device/attachments/room_browser_agent/message_4/attachment_4"
        )
      );
      hostedDevice.state.app.messages.push(
        hostedImageMessage("Image returned by agent.", false, 4, "agent-proof.png")
      );
      hostedDevice.emit();
      const agentImage = page.getByRole("img", { name: "agent-proof.png" });
      await agentImage.waitFor({ state: "visible" });
      await agentImage.evaluate((image) => image.scrollIntoView({ block: "center" }));
      const attachmentResponse = await agentAttachmentResponse;
      assert.equal(
        attachmentResponse.status(),
        200,
        `agent attachment download returned ${attachmentResponse.status()}`
      );
      await waitFor(() =>
        agentImage.evaluate(
          (image) =>
            image instanceof HTMLImageElement && image.complete && image.naturalWidth > 0
        )
      );
      assert.equal(
        hostedDevice.state.authRequests.some((request) =>
          request.path.startsWith("/v1/app/attachments/")
        ),
        true,
        "attachment bytes must traverse the authenticated hosted-device route"
      );

      hostedDevice.state.app.messages.push(
        hostedPlayableMessage("Video returned by agent.", 5, "agent-proof.mp4", "Video", "video/mp4")
      );
      hostedDevice.state.app.messages.push(
        hostedPlayableMessage("Audio returned by agent.", 6, "agent-proof.mp3", "VoiceNote", "audio/mpeg")
      );
      hostedDevice.emit();
      const video = page.locator('video[aria-label="agent-proof.mp4"]');
      const audio = page.locator('audio[aria-label="agent-proof.mp3"]');
      await video.waitFor({ state: "visible" });
      await audio.waitFor({ state: "visible" });
      assert.equal(await video.getAttribute("controls"), "");
      assert.equal(await audio.getAttribute("controls"), "");

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
        ...hostedMessage("💻 Running browser QA", false, 7),
        kind: "tool",
        status: "running",
      });
      hostedDevice.emit();
      await expectVisibleText(page, "Working · 1 step");

      hostedDevice.state.app.messages[hostedDevice.state.app.messages.length - 1]!.status =
        "complete";
      hostedDevice.state.app.messages.push({
        ...hostedMessage("Browser QA complete.", false, 8),
        final_delivery: true,
      });
      hostedDevice.emit();
      await expectVisibleText(page, "Worked through 1 step");
      await page
        .getByText("Completed Oslo Bot is working", { exact: true })
        .waitFor({ state: "hidden", timeout: 15_000 });

      await page.getByLabel("Message your agent").fill("Working lease browser proof.");
      await page.getByRole("button", { name: "Send message" }).click();
      await expectVisibleText(page, "Working lease browser proof.");
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
      hostedDevice.emit();
      await expectVisibleText(page, "Completed Oslo Bot is working");
      await page
        .getByText("Completed Oslo Bot is working", { exact: true })
        .waitFor({ state: "hidden", timeout: 20_000 });

      const localSiteUrl = sites.siteUrl;
      hostedDevice.state.app.messages.push(
        hostedMessage("Repository: https://git.finite.chat/browser-proof.git", false, 10)
      );
      hostedDevice.state.app.messages.push(
        hostedMessage(`Published your site: ${localSiteUrl}`, false, 11)
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

      const binding = hostedDevice.state.app.hosted_agent_binding;
      assert(binding);
      binding.associated_room_ids = ["room_browser_legacy"];
      hostedDevice.state.app.rooms.push({
        room_id: "room_browser_legacy",
        display_name: "Previous room",
        state: "Connected",
        status: "Connected",
        user_status_text: "Connected",
        last_message_preview: "Old chat",
        unread_count: 0,
        is_agent_chat: true,
      });
      hostedDevice.state.app.topics.push({
        room_id: "room_browser_legacy",
        topic_id: "topic_browser_legacy",
        title: "Previous topic",
        active_chat_id: "chat_browser_legacy",
        chats: [{
          chat_id: "chat_browser_legacy",
          title: "Old chat",
          active: true,
        }],
      });
      hostedDevice.state.app.messages.push({
        ...hostedMessage("Legacy room transcript only.", false, 12),
        room_id: "room_browser_legacy",
        message_id: "message_legacy_only",
        conversation_id: "topic_browser_legacy",
        chat_id: "chat_browser_legacy",
      });
      hostedDevice.state.app.selected_room_id = "room_browser_legacy";
      hostedDevice.state.app.selected_topic_id = "topic_browser_legacy";
      hostedDevice.state.app.selected_chat_id = "chat_browser_legacy";
      hostedDevice.emit();
      await waitFor(async () =>
        (await page.getByText("Previous conversations", { exact: true }).count()) === 0
        && (await page.getByRole("button", { name: "Previous topic", exact: true }).count()) === 0
      );
      await page
        .locator(".finite-chat__topbar")
        .getByText("General", { exact: true })
        .waitFor({ state: "visible" });
      await page
        .locator(".finite-chat__topbar")
        .getByText("Browser QA", { exact: true })
        .waitFor({ state: "visible" });
      assert.equal(await page.getByText("Legacy room transcript only.", { exact: true }).count(), 0);

      const actionsBeforeCanonicalReply = hostedDevice.state.actions.length;
      const canonicalReply = "Associated rooms cannot capture this reply.";
      await page.getByLabel("Message your agent").fill(canonicalReply);
      await page.getByRole("button", { name: "Send message" }).click();
      await waitFor(() => hostedDevice.state.actions
        .slice(actionsBeforeCanonicalReply)
        .some((action) => actionName(action) === "SendChatMessage"));
      const canonicalSend = hostedDevice.state.actions
        .slice(actionsBeforeCanonicalReply)
        .find((action) => actionName(action) === "SendChatMessage");
      assert.deepEqual(canonicalSend, {
        SendChatMessage: {
          room_id: "room_browser_agent",
          topic_id: "topic_browser_agent",
          chat_id: "chat_browser_agent",
          text: canonicalReply,
        },
      });
      await page
        .locator(".finite-chat__message")
        .getByText(canonicalReply, { exact: true })
        .waitFor({ state: "visible", timeout: 15_000 });

      await page.getByRole("button", { name: "New chat", exact: true }).click();
      await waitFor(() => hostedDevice.state.newChatRequests.length === 1);
      assert.deepEqual(hostedDevice.state.newChatRequests[0], {
        project_id: "project_running",
        room_id: "room_browser_agent",
        topic_id: "topic_browser_agent",
        reason: null,
        intent_key: hostedDevice.state.newChatRequests[0]!.intent_key,
      });
      assert.match(String(hostedDevice.state.newChatRequests[0]!.intent_key), /.+/);
      assert.equal(hostedDevice.state.app.selected_room_id, "room_browser_agent");
      assert.equal(hostedDevice.state.app.selected_topic_id, "topic_browser_agent");
      assert(
        hostedDevice.state.app.topics
          .find((topic) => topic.room_id === "room_browser_agent")
          ?.chats.some((chat) => chat.chat_id === "chat_browser_new_1")
      );

      hostedDevice.state.app.messages.push({
        ...hostedMessage("Remembered transcript only.", false, 13),
        message_id: "message_remembered_only",
        chat_id: "chat_browser_remembered",
      });
      await page
        .getByRole("button", { name: "Remembered work", exact: true })
        .click();
      await page
        .locator(".finite-chat__topbar")
        .getByText("Remembered work", { exact: true })
        .waitFor({ state: "visible" });
      await expectVisibleText(page, "Remembered transcript only.");
      // The pinned selection updates the pane before the daemon confirms, so
      // wait for the daemon-side selection rather than asserting it directly.
      await waitFor(
        () => hostedDevice.state.app.selected_chat_id === "chat_browser_remembered",
        5_000,
        () => "the daemon never persisted the Remembered work selection"
      );

      // Hold a selection-only OpenChat while a newer stream revision lands.
      // The clicked selection is pinned client-side immediately, so the pane
      // switches at once, the stale-selection stream snapshot applies its
      // content without yanking the selection back, and the equal-revision
      // mutation response still triggers a full state refetch to reconcile.
      const stateFetchesBeforeSelectionRace = hostedDevice.state.authRequests.filter(
        (request) => request.path === "/v1/app/agent-bindings/open"
      ).length;
      hostedDevice.holdNextNavigationAction();
      await page
        .locator(".finite-chat__folder-body")
        .getByRole("button", { name: "Browser QA", exact: true })
        .click();
      await page
        .locator(".finite-chat__topbar")
        .getByText("Browser QA", { exact: true })
        .waitFor({ state: "visible" });
      await waitFor(
        () => hostedDevice.state.navigationActionGate === null,
        5_000,
        () => "the selection race request did not reach the daemon"
      );
      hostedDevice.state.app.messages.push({
        ...hostedMessage("Concurrent stream update.", false, 14),
        message_id: "message_selection_race",
        chat_id: "chat_browser_agent",
      });
      hostedDevice.emit();
      // The stream snapshot still carries the previous selection while the
      // OpenChat is held; its message is only visible if the pinned Browser QA
      // pane stayed put instead of fighting back to Remembered work.
      await expectVisibleText(page, "Concurrent stream update.");
      hostedDevice.releaseNavigationAction();
      await waitFor(
        () =>
          hostedDevice.state.authRequests.filter(
            (request) => request.path === "/v1/app/agent-bindings/open"
          ).length > stateFetchesBeforeSelectionRace,
        5_000,
        () => "a rejected selection response did not trigger state reconciliation"
      );
      await page
        .locator(".finite-chat__topbar")
        .getByText("Browser QA", { exact: true })
        .waitFor({ state: "visible" });
      assert.equal(hostedDevice.state.app.selected_chat_id, "chat_browser_agent");

      await page
        .getByRole("button", { name: "Remembered work", exact: true })
        .click();
      await page
        .locator(".finite-chat__topbar")
        .getByText("Remembered work", { exact: true })
        .waitFor({ state: "visible" });
      // The pinned selection presents instantly; let the daemon confirm before
      // reading mutation counters so the next section starts quiescent.
      await waitFor(
        () => hostedDevice.state.app.selected_chat_id === "chat_browser_remembered",
        5_000,
        () => "the daemon never persisted the Remembered work selection"
      );

      // New chat also changes the selected Room/Topic/Chat. Hold it before it
      // mutates the fake daemon, then click an existing chat. If New chat is
      // outside the navigation lane, that later click arrives first and the
      // delayed New chat becomes the daemon's final state.
      const completedSelectionMutations =
        hostedDevice.state.completedSelectionMutations;
      const startedNavigationActions = hostedDevice.state.actions.filter(
        (action) => ["OpenRoom", "OpenTopic", "OpenChat"].includes(actionName(action))
      ).length;
      hostedDevice.holdNextNavigationAction();
      await page
        .getByRole("button", { name: "New chat in General", exact: true })
        .click();
      await waitFor(
        () => hostedDevice.state.navigationActionGate === null,
        5_000,
        () => "the first rapid navigation request did not reach the daemon"
      );
      await page
        .getByRole("button", { name: "Remembered work", exact: true })
        .click();
      // Under the buggy concurrent implementation the second request reaches
      // the daemon during this window. The fixed navigation lane intentionally
      // leaves it queued until the first request completes.
      await waitFor(
        () =>
          hostedDevice.state.actions.filter(
            (action) => ["OpenRoom", "OpenTopic", "OpenChat"].includes(actionName(action))
          ).length === startedNavigationActions + 1,
        750
      ).catch(() => undefined);
      hostedDevice.releaseNavigationAction();
      await waitFor(
        () =>
          hostedDevice.state.completedSelectionMutations
            === completedSelectionMutations + 2,
        5_000,
        () => `New chat and the subsequent navigation did not both finish (completed=${hostedDevice.state.completedSelectionMutations}, expected=${completedSelectionMutations + 2})`
      );
      assert.equal(
        hostedDevice.state.app.selected_chat_id,
        "chat_browser_remembered",
        "delayed network arrival persisted an older click over the user's last navigation intent"
      );
      await page
        .locator(".finite-chat__topbar")
        .getByText("Remembered work", { exact: true })
        .waitFor({ state: "visible" });
      await expectVisibleText(page, "Remembered transcript only.");

      const bindingOpensBeforeReturn = hostedDevice.state.authRequests.filter(
        (request) => request.path === "/v1/app/agent-bindings/open"
      ).length;
      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/completed-oslo-bot`
      );
      await page.getByRole("main").getByRole("link", { name: "Open chat" }).click();
      await page.waitForURL(/\/dashboard\/machines\/runtime_completed-oslo-bot\/chat$/u);
      await waitFor(
        () =>
          hostedDevice.state.authRequests.filter(
            (request) => request.path === "/v1/app/agent-bindings/open"
          ).length > bindingOpensBeforeReturn,
        5_000,
        () => "returning to chat did not reopen the canonical Agent binding"
      );
      await page
        .locator(".finite-chat__topbar")
        .getByText("Remembered work", { exact: true })
        .waitFor({ state: "visible" });
      assert.equal(
        hostedDevice.state.app.selected_chat_id,
        "chat_browser_remembered",
        "canonical binding reopen reset the remembered chat"
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
          agentRuntimeId: "runtime_removable-kata-bot",
        }),
      ],
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/removable-kata-bot`
      );
      await page.waitForURL(/\/dashboard\/machines\/runtime_removable-kata-bot$/u);
      assert.equal(
        await page.getByRole("button", { name: "Remove agent" }).count(),
        0,
        "ordinary destroy must stay hidden without an explicit Runtime Retirement capability"
      );
      assert.deepEqual(core.state.destroyPosts, []);
    });
  } finally {
    await browser?.close().catch(() => {});
    dashboard.kill("SIGTERM");
    paidDashboard.kill("SIGTERM");
    core.server.close();
    hostedDevice.close();
    brain.server.close();
    sites.server.close();
    await Promise.race([
      once(dashboard, "exit"),
      new Promise((resolve) => setTimeout(resolve, 2_000)),
    ]);
    await Promise.race([
      once(paidDashboard, "exit"),
      new Promise((resolve) => setTimeout(resolve, 2_000)),
    ]);
  }
});

function startDashboard(
  port: number,
  coreUrl: string,
  hostedDeviceUrl: string,
  brainUrl: string,
  sitesUrl: string,
  options: { stripeConfigured?: boolean; distDir?: string } = {}
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
        FC_DASHBOARD_RUNTIME_MODE: options.stripeConfigured ? "customer" : "canary",
        WORKOS_COOKIE_PASSWORD: "browser-test-cookie-password-32-characters-minimum",
        FC_WORKOS_AUTH_ENABLED: "0",
        NEXT_PUBLIC_WORKOS_REDIRECT_URI: `http://127.0.0.1:${port}/callback`,
        STRIPE_SECRET_KEY: options.stripeConfigured ? "sk_test_browser_fixture" : "",
        STRIPE_FINITE_COMPUTER_STANDARD_PRICE_ID: options.stripeConfigured
          ? "price_browser_fixture"
          : "",
        STRIPE_WEBHOOK_SECRET: options.stripeConfigured ? "whsec_browser_fixture" : "",
        FC_DASHBOARD_BASE_URL: options.stripeConfigured
          ? `http://127.0.0.1:${port}`
          : "",
        FC_DASHBOARD_PUBLIC_URL: "",
        NEXT_PUBLIC_APP_URL: "",
        NEXT_DIST_DIR: options.distDir ?? ".next-browser-test",
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
    const requestUrl = new URL(request.url ?? "/", "http://brain.test");
    if (request.method === "GET" && requestUrl.pathname === "/client") {
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      response.end(`<!doctype html><html><head></head><body><main><h1>FiniteBrain browser proof</h1><p>First-party client origin reached.</p><p id="api-status">Connecting…</p></main><script>fetch('/_admin/browser-proof').then((result) => result.json()).then((result) => { document.getElementById('api-status').textContent = result.message; });</script></body></html>`);
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
  const app = initialHostedChatState();
  const state: HostedDeviceState = {
    unavailable: false,
    ownerClaimGate: null,
    releaseOwnerClaimGate: null,
    navigationActionGate: null,
    releaseNavigationActionGate: null,
    completedSelectionMutations: 0,
    app,
    actions: [],
    newChatRequests: [],
    runtimeCommands: [],
    authRequests: [],
    bindingAuthorizations: [
      {
        project_id: "project_second",
        creation_request_id: "agent_request_second",
      },
    ],
    bindingAuthorizationFailuresRemaining: 0,
    agentBindings: new Map(),
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
    holdOwnerClaim() {
      if (state.ownerClaimGate) {
        throw new Error("owner claim is already held");
      }
      state.ownerClaimGate = new Promise((resolve) => {
        state.releaseOwnerClaimGate = resolve;
      });
    },
    releaseOwnerClaim() {
      state.releaseOwnerClaimGate?.();
      state.ownerClaimGate = null;
      state.releaseOwnerClaimGate = null;
    },
    holdNextNavigationAction() {
      if (state.navigationActionGate) {
        throw new Error("a navigation action is already held");
      }
      state.navigationActionGate = new Promise((resolve) => {
        state.releaseNavigationActionGate = resolve;
      });
    },
    releaseNavigationAction() {
      state.releaseNavigationActionGate?.();
      state.releaseNavigationActionGate = null;
    },
    setAvailable(available: boolean) {
      state.unavailable = !available;
      if (!available) {
        for (const stream of streams) stream.end();
        streams.clear();
      }
    },
    emit() {
      state.app.rev += 1;
      emitHostedState(streams, state.app);
    },
    failNextBindingAuthorization() {
      state.bindingAuthorizationFailuresRemaining += 1;
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

  if (state.unavailable) {
    writeJson(response, 503, { error: "hosted chat is temporarily unavailable" });
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
    if (command.command === "agent.owner.claim" && state.ownerClaimGate) {
      await state.ownerClaimGate;
    }
    writeJson(response, 200, applyRuntimeCommand(state, command));
    return;
  }

  if (request.method === "POST" && path === "/v1/app/agent-bindings/open") {
    const body = (await readJson(request)) as Record<string, unknown>;
    const projectId = String(body.project_id ?? "");
    const binding = state.agentBindings.get(projectId);
    if (!binding) {
      writeJson(response, 404, { error: "agent binding not found" });
      return;
    }
    state.app.hosted_agent_binding = binding;
    writeJson(response, 200, state.app);
    return;
  }

  if (
    request.method === "POST" &&
    path === "/v1/app/agent-bindings/authorize-bootstrap"
  ) {
    const body = (await readJson(request)) as Record<string, unknown>;
    const authorization = {
      project_id: String(body.project_id ?? ""),
      creation_request_id: String(body.creation_request_id ?? ""),
    };
    assert(authorization.project_id);
    assert(authorization.creation_request_id);
    state.bindingAuthorizations.push(authorization);
    if (state.bindingAuthorizationFailuresRemaining > 0) {
      state.bindingAuthorizationFailuresRemaining -= 1;
      writeJson(response, 503, {
        error: "binding authorization is temporarily unavailable",
      });
      return;
    }
    writeJson(response, 200, { status: "authorized" });
    return;
  }

  if (request.method === "POST" && path === "/v1/app/agent-bindings/ensure") {
    const body = (await readJson(request)) as Record<string, unknown>;
    const projectId = String(body.project_id ?? "");
    assert(projectId);
    let binding = state.agentBindings.get(projectId);
    if (!binding) {
      if (
        !state.bindingAuthorizations.some(
          (authorization) => authorization.project_id === projectId
        )
      ) {
        writeJson(response, 409, {
          error:
            "first-time binding bootstrap was not authorized by Project creation",
        });
        return;
      }
      applyHostedAction(state.app, { StartProfileChat: null });
      binding = {
        version: 1,
        project_id: projectId,
        human_account_id: state.app.identity.account_id,
        agent_account_id: "agent-account-browser",
        agent_npub: String(body.agent_npub ?? AGENT_NPUB),
        canonical_room_id: "room_browser_agent",
        associated_room_ids: [],
      };
      state.agentBindings.set(projectId, binding);
    }
    state.app.hosted_agent_binding = binding;
    writeJson(response, 200, state.app);
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

  if (request.method === "POST" && path === "/v1/app/new-chat") {
    const body = (await readJson(request)) as Record<string, unknown>;
    state.newChatRequests.push(body);
    assert.equal(body.project_id, "project_running");
    assert.equal(body.room_id, "room_browser_agent");
    assert.equal(body.topic_id, "topic_browser_agent");
    if (state.navigationActionGate) {
      const gate = state.navigationActionGate;
      state.navigationActionGate = null;
      await gate;
    }
    applyHostedAction(state.app, {
      StartTopicChatIntent: {
        room_id: body.room_id,
        topic_id: body.topic_id,
        reason: body.reason,
        intent_key: body.intent_key,
      },
    });
    state.completedSelectionMutations += 1;
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
    const navigationAction = ["OpenRoom", "OpenTopic", "OpenChat"].includes(
      actionName(action)
    );
    state.actions.push(action);
    if (navigationAction && state.navigationActionGate) {
      const gate = state.navigationActionGate;
      state.navigationActionGate = null;
      await gate;
    }
    applyHostedAction(state.app, action);
    if (navigationAction) state.completedSelectionMutations += 1;
    if (!["OpenChat", "OpenTopic"].includes(actionName(action))) {
      emitHostedState(streams, state.app);
    }
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
    devices: [
      {
        account_id: "browser-user-account",
        device_id: "hosted-web",
        active: true,
        current_device: true,
        revoked: false,
        room_count: 1,
      },
      {
        account_id: "browser-user-account",
        device_id: "electron-browser-proof",
        active: true,
        current_device: false,
        revoked: false,
        room_count: 1,
      },
    ],
    typing_members: [],
    hosted_agent_binding: null,
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
    if (!state.rooms.some((room) => room.room_id === "room_browser_agent")) {
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
            {
              chat_id: "chat_browser_remembered",
              title: "Remembered work",
              active: false,
            },
          ],
        },
      ];
      state.messages = [hostedMessage("Hello from Completed Oslo Bot.", false, 1)];
    }
    const selectedTopic = state.selected_room_id === "room_browser_agent"
      ? state.topics.find(
          (topic) =>
            topic.room_id === "room_browser_agent"
            && topic.topic_id === state.selected_topic_id
        )
      : undefined;
    const selectedChatStillExists = selectedTopic?.chats.some(
      (chat) => chat.chat_id === state.selected_chat_id
    );
    state.selected_room_id = "room_browser_agent";
    if (!selectedChatStillExists) {
      state.selected_topic_id = "topic_browser_agent";
      state.selected_chat_id = "chat_browser_agent";
    }
  } else if (operation === "OpenChat") {
    const payload = action.OpenChat as Record<string, unknown> | undefined;
    assert(payload);
    const roomId = String(payload.room_id ?? "");
    const topicId = String(payload.topic_id ?? "");
    const chatId = String(payload.chat_id ?? "");
    const topic = state.topics.find(
      (candidate) => candidate.room_id === roomId && candidate.topic_id === topicId
    );
    const chat = topic?.chats.find((candidate) => candidate.chat_id === chatId);
    assert(topic && chat);
    state.selected_room_id = roomId;
    state.selected_topic_id = topicId;
    state.selected_chat_id = chatId;
    state.topics = state.topics.map((candidate) =>
      candidate.room_id === roomId && candidate.topic_id === topicId
        ? {
            ...candidate,
            active_chat_id: chatId,
            chats: candidate.chats.map((candidateChat) => ({
              ...candidateChat,
              active: candidateChat.chat_id === chatId,
            })),
          }
        : candidate
    );
  } else if (operation === "OpenTopic") {
    const payload = action.OpenTopic as Record<string, unknown> | undefined;
    assert(payload);
    const roomId = String(payload.room_id ?? "");
    const topicId = String(payload.topic_id ?? "");
    const topic = state.topics.find(
      (candidate) => candidate.room_id === roomId && candidate.topic_id === topicId
    );
    assert(topic);
    const chatId = topic.active_chat_id ?? topic.chats[0]?.chat_id ?? null;
    state.selected_room_id = roomId;
    state.selected_topic_id = topicId;
    state.selected_chat_id = chatId;
  } else if (operation === "StartTopicChatIntent") {
    const payload = action.StartTopicChatIntent as Record<string, unknown> | undefined;
    assert(payload);
    const roomId = String(payload.room_id ?? "");
    const topicId = String(payload.topic_id ?? "");
    const topic = state.topics.find(
      (candidate) => candidate.room_id === roomId && candidate.topic_id === topicId
    );
    assert(topic);
    const chatId = `chat_browser_new_${topic.chats.filter((chat) =>
      chat.chat_id.startsWith("chat_browser_new_")
    ).length + 1}`;
    topic.chats.push({ chat_id: chatId, title: "New chat", active: true });
    topic.active_chat_id = chatId;
    state.selected_room_id = roomId;
    state.selected_topic_id = topicId;
    state.selected_chat_id = chatId;
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
  } else if (operation === "RevokeDevice") {
    const payload = action.RevokeDevice as Record<string, unknown> | undefined;
    const device = state.devices.find(
      (candidate) =>
        candidate.account_id === payload?.account_id
        && candidate.device_id === payload?.device_id
    );
    assert(device && !device.current_device);
    device.active = false;
    device.revoked = true;
  }
  if (operation !== "OpenChat") state.rev += 1;
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

function hostedPlayableMessage(
  text: string,
  seq: number,
  filename: string,
  kind: "VoiceNote" | "Video",
  mimeType: string
): FakeHostedChatState["messages"][number] {
  return {
    ...hostedMessage(text, false, seq),
    kind: "media",
    media: [
      {
        attachment_id: `attachment_${seq}`,
        mime_type: mimeType,
        filename,
        kind,
        width: kind === "Video" ? 640 : null,
        height: kind === "Video" ? 360 : null,
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

async function startFakeCore(onRecover: () => void = () => {}) {
  let state = emptyCoreState();
  const server = http.createServer(async (request, response) => {
    try {
      await handleCoreRequest(request, response, state, onRecover);
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
  state: CoreState,
  onRecover: () => void
) {
  if (
    request.headers.authorization !== `Bearer ${CORE_TOKEN}` &&
    request.headers.authorization !== "Bearer fixture-browser-access-token"
  ) {
    writeJson(response, 401, { error: "missing service token" });
    return;
  }

  if (request.method === "GET" && request.url === "/api/core/v1/me") {
    state.meGets += 1;
    writeJson(response, 200, {
      email: "browser@finite.vip",
      workos_user_id: "user_browser",
      claimable_candidates: [],
      projects: state.projects,
      agent_creation_requests: state.requests,
    });
    return;
  }

  const runtimeRouteMatch = request.url?.match(
    /^\/api\/core\/v1\/me\/runtime-routes\/([^/]+)$/u
  );
  if (request.method === "GET" && runtimeRouteMatch?.[1]) {
    state.runtimeRouteGets += 1;
    const identifier = decodeURIComponent(runtimeRouteMatch[1]);
    const project = state.runtimeRouteProjectIdOverride
      ? state.projects.find(
          (candidate) => candidate.project.id === state.runtimeRouteProjectIdOverride
        )
      : state.projects.find(
      (candidate) =>
        candidate.project.id === identifier ||
        candidate.runtime?.id === identifier ||
        candidate.runtime?.id === `runtime_${identifier}`
        );
    if (!project?.runtime) {
      writeJson(response, 404, { error: "agent runtime was not found" });
      return;
    }
    writeJson(response, 200, {
      project_id: project.project.id,
      runtime_id: project.runtime.id,
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
      requires_billing: state.requiresBilling,
    });
    return;
  }

  const runtimeControlMatch = request.url?.match(
    /^\/api\/core\/v1\/me\/projects\/([^/]+)\/runtime\/(restart|recover-known-good-chat)$/u
  );
  if (request.method === "POST" && runtimeControlMatch?.[1] && runtimeControlMatch[2]) {
    const projectId = decodeURIComponent(runtimeControlMatch[1]);
    const kind = runtimeControlMatch[2];
    const project = state.projects.find((candidate) => candidate.project.id === projectId);
    if (kind === "restart") {
      state.restartPosts.push(projectId);
    } else {
      state.recoverPosts.push(projectId);
      onRecover();
    }
    writeJson(response, 200, {
      id: `runtime_control_${kind}_${state.restartPosts.length + state.recoverPosts.length}`,
      project_id: projectId,
      agent_runtime_id: project?.runtime?.id ?? "runtime_missing",
      source_host_id: "browser-fixture",
      source_machine_id: "internal-browser-runtime",
      requested_by_user_id: "user_browser",
      kind: kind === "restart" ? "restart" : "recover_known_good_chat_runtime",
      status: "requested",
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
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
    const idempotencyKey = String(body.idempotencyKey ?? "");
    const existingResult = state.creationResults.get(idempotencyKey);
    const resultIdentity = existingResult ?? {
      projectId: `project_${state.creationResults.size + 1}`,
      requestId: `agent_request_${state.creationResults.size + 1}`,
    };
    state.creationResults.set(idempotencyKey, resultIdentity);
    const projectId = resultIdentity.projectId;
    const requestRecord = agentCreationRequest({
      id: resultIdentity.requestId,
      projectId,
      displayName: String(body.displayName ?? "Oslo Bot"),
      status: "requested",
      createdAt: new Date().toISOString(),
    });
    if (!state.projects.some((candidate) => candidate.project.id === projectId)) {
      state.projects.push({
        project: {
          id: projectId,
          display_name: requestRecord.display_name,
          created_at: requestRecord.created_at,
          updated_at: requestRecord.updated_at,
        },
        runtime: null,
      });
    }
    state.requests = [requestRecord];
    writeJson(response, 200, {
      reused: Boolean(existingResult),
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
      source_host_id: "browser-fixture",
      source_machine_id: "internal-browser-runtime",
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
    creationResults: new Map(),
    meGets: 0,
    runtimeRouteGets: 0,
    runtimeRouteProjectIdOverride: null,
    cancelPosts: [],
    destroyPosts: [],
    recoverPosts: [],
    restartPosts: [],
    createDelayMs: 0,
    canCreateAgent: false,
    requiresBilling: true,
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
  agentRuntimeId = null,
}: {
  id: string;
  projectId: string;
  displayName?: string;
  status: AgentCreationRequest["status"];
  failureMessage?: string | null;
  createdAt?: string;
  agentRuntimeId?: string | null;
}): AgentCreationRequest {
  return {
    id,
    project_id: projectId,
    display_name: displayName,
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
  legacyMachineId = "completed-oslo-bot",
  runtimeRetirement = false,
  recoverKnownGoodChat = false
): VisibleProject {
  return {
    project: {
      id: projectId,
      display_name: displayName,
      created_at: "2026-05-28T12:00:00Z",
      updated_at: "2026-05-28T12:01:00Z",
    },
    runtime: {
      id: `runtime_${legacyMachineId}`,
      project_id: projectId,
      contact_endpoint: runtimeStatusUrl,
      runtime_status: "online",
      hermes_available: true,
      runtime_capabilities: {
        restart: true,
        recover_known_good_chat: recoverKnownGoodChat,
        runtime_upgrade: false,
        stop: true,
        runtime_retirement: runtimeRetirement,
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
