import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import http, { type IncomingMessage, type ServerResponse } from "node:http";
import { once } from "node:events";
import { mkdir, rm } from "node:fs/promises";
import { test } from "node:test";

import { chromium, type Browser, type Page } from "playwright";

import { chromiumLaunchOptions } from "../scripts/playwright-browser";
import { sealAgentOnboardingDraft } from "../src/lib/agent-onboarding";

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
  is_relocation: boolean;
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
  meError: boolean;
  runtimeRouteGets: number;
  runtimeRouteProjectIdOverride: string | null;
  cancelPosts: string[];
  destroyPosts: string[];
  recoverPosts: string[];
  restartPosts: string[];
  createDelayMs: number;
  canCreateAgent: boolean;
  requiresBilling: boolean;
  billingClass: "sponsored" | "standard";
  billingError: boolean;
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
      archived: boolean;
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
      url?: string | null;
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
  updatesUnavailable: boolean;
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
  directImageGets: number;
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
    serviceAuthorization: string | null;
    outputUrl: string;
    verifiedEmail: string;
    returnTo: string;
  }>;
  redemptions: number;
  privateContentRequests: number;
};

test("dashboard agent creation browser states", { timeout: 300_000 }, async () => {
  await resetDashboardDevDirs();
  const hostedDevice = await startFakeHostedDevice();
  const core = await startFakeCore(() => hostedDevice.setAvailable(true));
  const sites = await startFakeSites();
  let dashboard: ChildProcessWithoutNullStreams | null = null;
  let dashboardOutput = () => "";
  const paidDashboardPort = await freePort();
  const paidDashboard = startDashboard(
    paidDashboardPort,
    core.url,
    hostedDevice.url,
    sites.apiUrl,
    {
      admin: true,
      stripeConfigured: true,
      runtimeRetirement: true,
      distDir: ".next-browser-stripe-test",
    }
  );
  const paidDashboardOutput = collectOutput(paidDashboard);
  let browser: Browser | null = null;

  try {
    await waitForDashboard(paidDashboardPort, paidDashboardOutput);
    browser = await chromium.launch({
      headless: true,
      args: [
        "--use-fake-device-for-media-stream",
        "--use-fake-ui-for-media-stream",
      ],
      ...chromiumLaunchOptions(),
    });

    core.reset({
      canCreateAgent: true,
      requiresBilling: false,
    });
    await withSignedInPage(browser, paidDashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${paidDashboardPort}/dashboard?new=1`);
      await page.getByRole("heading", { name: "Do you have a launch code?" }).waitFor();
      await onboardingScreenshots(page, "code");
      assert.equal(await page.getByRole("button", { name: "Use account access" }).count(), 0,
        "customer mode must still require explicit access despite stale capacity");
      await page.getByRole("button", { name: "Continue without a code" }).click();
      await page.getByRole("heading", { name: "Your own Finite Agent." }).waitFor();
      await expectVisibleText(page, "Hosted Agent");
      await expectVisibleText(page, "$200 USD");
      await onboardingScreenshots(page, "plan");
      await page.getByText("Renews monthly until you cancel in the billing portal.").waitFor();
      await page.getByText("refunds are handled per our").waitFor();
      assert.equal(core.state.creationPosts.length, 0, "reviewing the plan must not create an agent");
      await page.getByRole("button", { name: "Continue", exact: true }).click();
      await page.getByLabel("Agent name").fill("Customer Access Proof");
      await onboardingScreenshots(page, "name");
      await page.getByRole("button", { name: "Continue to secure payment" }).waitFor();
      assert.equal(core.state.creationPosts.length, 0, "naming must not create an unpaid agent");
      await page.getByRole("button", { name: "Back", exact: true }).click();
      await page.getByRole("button", { name: "Back", exact: true }).click();
      await page.getByRole("radio", { name: /Confidential/u }).check();
      await expectVisibleText(page, "Enter a Confidential Launch Code. Standard subscriptions do not unlock this option yet.");
      assert.equal(await page.getByRole("button", { name: "Continue without a code" }).count(), 0);
      await page.getByRole("textbox", { name: "Confidential Launch Code" }).waitFor();

    });

    // A new dashboard must resume a signed draft written by the previous
    // dashboard release. Core's billing state remains the payment authority.
    core.reset({ billingClass: "standard", requiresBilling: true });
    await withSignedInPage(browser, paidDashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${paidDashboardPort}/dashboard?billing=success`);
      await expectVisibleText(page, "Confirming your payment");
      assert.equal(await page.getByRole("button", { name: "Continue without a code" }).count(), 0);
      assert.equal(core.state.creationPosts.length, 0);
      await page.goto(`http://127.0.0.1:${paidDashboardPort}/dashboard?billing=success&billingSyncStartedAt=${Date.now() - 120_000}`);
      await expectVisibleText(page, "Payment received, still syncing");
      assert.equal(core.state.creationPosts.length, 0);
      await page.goto(`http://127.0.0.1:${paidDashboardPort}/dashboard?billing=cancelled`);
      await expectVisibleText(page, "Checkout cancelled");
      await page.getByLabel("Launch Code", { exact: true }).waitFor();
    });
    core.reset({ billingClass: "standard", requiresBilling: false, canCreateAgent: true });
    await withSignedInPage(browser, paidDashboardPort, async (page) => {
      const sealed = await sealAgentOnboardingDraft({
        version: 1,
        workosUserId: "user_browser",
        displayName: "Paid Return Bot",
        profilePictureUrl: null,
        hostingTier: "standard",
        idempotencyKey: "browser-previous-dashboard-paid-draft",
        issuedAtMs: Date.now() - 10_000,
        stripeCheckoutStartedAtMs: Date.now() - 5_000,
      }, { WORKOS_COOKIE_PASSWORD: "browser-test-cookie-password-32-characters-minimum" });
      await page.context().addCookies([{
        name: "finite-agent-draft", value: sealed, domain: "127.0.0.1", path: "/dashboard", httpOnly: true, sameSite: "Lax",
      }]);
      core.state.billingError = true;
      await page.goto(`http://127.0.0.1:${paidDashboardPort}/dashboard?billing=success`);
      await page.getByRole("link", { name: "Retry agent setup", exact: true }).waitFor();
      assert.equal(await page.getByRole("button", { name: "Continue without a code" }).count(), 0);
      assert.equal(core.state.creationPosts.length, 0);
      core.state.billingError = false;
      hostedDevice.failNextBindingAuthorization();
      await page.getByRole("link", { name: "Retry agent setup", exact: true }).click();
      await page.getByRole("alert").filter({ hasText: "binding authorization is temporarily unavailable" }).waitFor();
      assert.equal(core.state.creationPosts.length, 1);
      assert.equal(core.state.creationResults.size, 1);
      assert.equal(await page.getByRole("button", { name: "Continue without a code" }).count(), 0);
      assert.equal(await page.getByRole("button", { name: "Continue to secure payment" }).count(), 0);
      core.state.canCreateAgent = false; // The first attempt used the last allowance.
      core.state.billingError = true;
      await page.getByRole("link", { name: "Retry agent setup", exact: true }).click();
      await page.getByRole("alert").filter({ hasText: "Billing is temporarily unavailable for browser proof." }).waitFor();
      assert.equal(core.state.creationPosts.length, 1);
      assert.equal(await page.getByRole("button", { name: "Continue without a code" }).count(), 0);
      core.state.billingError = false;
      await page.getByRole("link", { name: "Retry agent setup", exact: true }).click();
      await page.waitForURL(/creation=agent_request_1/u);
      assert.equal(core.state.creationPosts.length, 2);
      assert.equal(core.state.creationResults.size, 1, "paid retry must reuse the original Project");
      assert.equal((core.state.creationPosts[1] as Record<string, unknown>).idempotencyKey,
        (core.state.creationPosts[0] as Record<string, unknown>).idempotencyKey);
      const post = core.state.creationPosts[0] as Record<string, unknown>;
      assert.equal(post.displayName, "Paid Return Bot");
      assert.equal(post.launchCode, "");
      assert.equal(post.idempotencyKey, "browser-previous-dashboard-paid-draft");
      assert.equal((await page.context().cookies()).some((cookie) => cookie.name === "finite-agent-draft"), false);
      await page.reload();
      assert.equal(core.state.creationPosts.length, 2, "reloading a paid return must not duplicate creation");
    });

    core.reset({
      projects: [
        visibleProject(
          "project_advanced",
          "Advanced Controls Bot",
          hostedDevice.runtimeStatusUrl,
          "advanced-controls-bot",
          true,
          true
        ),
      ],
      requests: [
        agentCreationRequest({
          id: "agent_request_advanced",
          projectId: "project_advanced",
          displayName: "Advanced Controls Bot",
          status: "running",
          agentRuntimeId: "runtime_advanced-controls-bot",
        }),
      ],
    });
    await withSignedInPage(browser, paidDashboardPort, async (page) => {
      await page.goto(
        `http://127.0.0.1:${paidDashboardPort}/dashboard/machines/runtime_advanced-controls-bot`
      );
      const advanced = page.locator("summary").filter({ hasText: /^Advanced$/u });
      await advanced.waitFor({ state: "visible" }).catch(async (error) => {
        throw new Error(
          `Advanced controls did not render: ${String(error)}\n${await pageText(page)}\n${paidDashboardOutput()}`
        );
      });
      await page
        .getByRole("heading", { name: "Chat recovery" })
        .waitFor({ state: "hidden" });
      await page
        .getByRole("heading", { name: "Retire this agent" })
        .waitFor({ state: "hidden" });
      await advanced.click();
      await page
        .getByRole("button", { name: "Recover chat" })
        .waitFor({ state: "visible" });
      await page
        .getByRole("button", { name: "Retire agent" })
        .waitFor({ state: "visible" });
    });

    // This differently configured Next dev server is not used again. Keeping
    // both compilers alive makes later on-demand route compilation contend in CI.
    await stopChildProcess(paidDashboard);

    const dashboardPort = await freePort();
    dashboard = startDashboard(
      dashboardPort,
      core.url,
      hostedDevice.url,
      sites.apiUrl,
      {
        runtimeRetirement: true,
      }
    );
    dashboardOutput = collectOutput(dashboard);
    await waitForDashboard(dashboardPort, dashboardOutput);

    core.reset({
      projects: [
        visibleProject(
          "project_non_admin_advanced",
          "Customer Controls Bot",
          hostedDevice.runtimeStatusUrl,
          "customer-controls-bot",
          true,
          true
        ),
      ],
      requests: [],
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/runtime_customer-controls-bot`
      );
      await page.getByRole("heading", { name: "Customer Controls Bot" }).waitFor({
        state: "visible",
      });
      assert.equal(
        await page.locator("summary").filter({ hasText: /^Advanced$/u }).count(),
        0,
        "non-admin viewers must not see Advanced runtime controls"
      );
      assert.equal(await page.getByRole("button", { name: "Recover chat" }).count(), 0);
      assert.equal(await page.getByRole("button", { name: "Retire agent" }).count(), 0);
    });

    core.reset({ meError: true });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await page.getByRole("heading", { name: "Could not load your account" }).waitFor();
      assert.equal(await page.getByLabel("Launch Code", { exact: true }).count(), 0);
      assert.equal(core.state.creationPosts.length, 0);
    });
    const runtimeLessProject = visibleProject("project_retained", "Retained Agent", hostedDevice.runtimeStatusUrl);
    runtimeLessProject.runtime = null;
    core.reset({
      projects: [runtimeLessProject],
      requests: [agentCreationRequest({
        id: "retired_request", projectId: "project_retained", status: "running",
        agentRuntimeId: "runtime_retired",
      })],
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await page.getByRole("heading", { name: "Retained Agent", exact: true }).waitFor();
      assert.equal(await page.getByLabel("Launch Code", { exact: true }).count(), 0);
      assert.equal(core.state.creationPosts.length, 0);
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard?new=1`);
      await page.getByLabel("Launch Code", { exact: true }).waitFor({ timeout: 10_000 });
      assert.equal(await page.getByText("Preparing your workspace", { exact: true }).count(), 0);
      assert.equal(core.state.creationPosts.length, 0, "New agent must not replay the completed launch");
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard?creation=retired_request`);
      await expectVisibleText(page, "Preparing your workspace");
      assert.equal(await page.getByLabel("Launch Code", { exact: true }).count(), 0);
    });
    core.reset({ requests: [
      agentCreationRequest({ id: "ambiguous_1", projectId: "project_1", status: "running" }),
      agentCreationRequest({ id: "ambiguous_2", projectId: "project_2", status: "running" }),
    ] });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await expectVisibleText(page, "Preparing your workspace");
      assert.equal(new URL(page.url()).searchParams.has("creation"), false);
      assert.equal(await page.getByLabel("Launch Code", { exact: true }).count(), 0);
      assert.equal(core.state.creationPosts.length, 0);
    });

    core.reset();
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await prepareAgentLaunch(page, "Oslo Bot");
      await page.getByRole("button", { name: "Account menu" }).waitFor({ state: "visible" });
      assert.equal(
        await page.getByRole("radio", { name: /Confidential/u }).count(),
        0,
        "Confidential onboarding must remain hidden from non-admin customers"
      );
      await page.locator('#coreAgentPicture').setInputFiles({
        name: "oslo-bot.png",
        mimeType: "image/png",
        buffer: PNG_BYTES,
      });
      await page.getByRole("img", { name: "Agent profile preview" }).waitFor({ state: "visible" });
      assert.equal(
        await page.getByRole("button", { name: "Continue to secure payment" }).count(),
        0,
        "payment must stay hidden when the webhook/return path is not fully configured"
      );
      await page.getByRole("button", { name: "Launch agent", exact: true }).click();
      await waitFor(
        () => core.state.creationPosts.length === 1,
        5_000,
        async () => `agent creation POST was not sent\n${await pageText(page)}`
      );
      const post = core.state.creationPosts[0] as Record<string, unknown>;
      assert.equal(post.displayName, "Oslo Bot");
      assert.equal(post.launchCode, "fixture-launch-code");
      assert.equal(post.hostingTier, "standard");
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
      await prepareAgentLaunch(page, "Double Submit Bot");
      await page.getByRole("button", { name: "Launch agent", exact: true }).dblclick();
      await waitFor(() => core.state.creationPosts.length === 1);
      await new Promise((resolve) => setTimeout(resolve, 700));
      assert.equal(core.state.creationPosts.length, 1);
    });

    core.reset();
    hostedDevice.failNextBindingAuthorization();
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await prepareAgentLaunch(page, "Authorization Retry Bot");
      await page.getByRole("button", { name: "Launch agent", exact: true }).click();
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
      const retryCode = page.getByLabel("Launch Code", { exact: true });
      await retryCode.waitFor({ state: "visible" }).catch(async (error) => {
        throw new Error(
          `binding authorization retry form did not render: ${String(error)}\n${await pageText(page)}`
        );
      });
      await expectVisibleText(page, "binding authorization is temporarily unavailable");
      await prepareAgentLaunch(page, "Authorization Retry Bot");
      await page.getByRole("button", { name: "Launch agent", exact: true }).click();
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
      await prepareAgentLaunch(page, "Fresh Code Bot", "fresh-top-up-code");
      await page.getByRole("button", { name: "Launch agent", exact: true }).click();
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

    core.reset({ requests: [agentCreationRequest({
      id: "request_no_runtime", projectId: "project_no_runtime", status: "running",
    })] });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await page.waitForURL(/creation=request_no_runtime/u);
      await expectVisibleText(page, "Preparing your workspace");
      assert.equal(await page.getByRole("link", { name: "Continue to chat" }).count(), 0);
      await onboardingScreenshots(page, "launch");
      core.state.projects = [visibleProject("project_no_runtime", "Oslo Bot", hostedDevice.runtimeStatusUrl, "untracked-bot")];
      await page.getByRole("heading", { name: "Oslo Bot is alive!", exact: true }).waitFor();
      assert.equal(core.state.creationPosts.length, 0, "resuming progress must not create an agent");
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
      requests: [
        agentCreationRequest({
          id: "agent_request_immersive_failed",
          projectId: "project_immersive_failed",
          displayName: "Failed Immersive Bot",
          status: "failed",
          failureMessage: "Runner failed its startup health check",
        }),
      ],
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard?new=1&creation=agent_request_immersive_failed`
      );
      await expectVisibleText(page, "Agent creation needs a retry");
      await expectVisibleText(page, "Runner failed its startup health check");
      assert.equal(
        (await page.locator('[aria-current="step"]').innerText()).trim(),
        "Launch: current"
      );

      await page.getByRole("button", { name: "Start over" }).click();
      await waitFor(() =>
        core.state.cancelPosts.includes("agent_request_immersive_failed")
      );
      await page.getByLabel("Launch Code", { exact: true }).waitFor({ state: "visible" });
      await expectVisibleText(page, "Do you have a launch code?");
      await waitFor(async () =>
        (await page.locator('[aria-current="step"]').innerText()).trim() ===
        "Launch code: current"
      );
    });

    core.reset({
      projects: [
        visibleProject(
          "project_relocation_failed",
          "Relocated Oslo Bot",
          hostedDevice.runtimeStatusUrl
        ),
      ],
      requests: [
        agentCreationRequest({
          id: "agent_request_relocation_failed",
          projectId: "project_relocation_failed",
          displayName: "Relocated Oslo Bot",
          status: "failed",
          failureMessage: "operator-only relocation failed",
          agentRuntimeId: "runtime_completed-oslo-bot",
          isRelocation: true,
        }),
      ],
    });
    await withSignedInPage(browser, dashboardPort, async (page) => {
      await page.goto(`http://127.0.0.1:${dashboardPort}/dashboard`);
      await expectVisibleText(page, "Relocated Oslo Bot");
      assert.equal(
        await page.getByText("Agent creation needs a retry", { exact: true }).count(),
        0
      );
      assert.equal(await page.getByText("operator-only relocation failed", { exact: true }).count(), 0);
      assert.equal(await page.getByRole("button", { name: "Start over" }).count(), 0);
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
      const brainLink = productNav.getByRole("link", { name: "Brain", exact: true });
      await brainLink.waitFor({ state: "visible" });
      assert.equal(
        await brainLink.getAttribute("href"),
        "/dashboard/machines/runtime_completed-oslo-bot/brain"
      );
      const skillsLink = productNav.getByRole("link", { name: "Skills", exact: true });
      await skillsLink.waitFor({ state: "visible" });
      assert.equal(
        await skillsLink.getAttribute("href"),
        "/dashboard/skills?machine=runtime_completed-oslo-bot"
      );
      await skillsLink.click();
      await page.waitForURL(/\/dashboard\/skills\?machine=runtime_completed-oslo-bot$/u);
      await page.getByRole("heading", { name: "Skills", exact: true }).waitFor({ state: "visible" });
      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/runtime_completed-oslo-bot`
      );
      await page
        .getByRole("navigation", { name: "Agent, topics, and chats" })
        .waitFor({ state: "visible" });

      const machineSwitcher = page.getByRole("button", {
        name: "Completed Oslo Bot, Online",
        exact: true,
      });
      await waitFor(
        async () => {
          if (await machineSwitcher.isVisible()) return true;
          await page.waitForTimeout(250);
          await page.reload();
          return machineSwitcher.isVisible();
        },
        15_000,
        async () =>
          `agent switcher did not hydrate after returning from Skills\nURL: ${page.url()}\n${await pageText(page)}\n${dashboardOutput()}`
      );
      await machineSwitcher.click();
      const secondAgentItem = page.getByRole("menuitem", {
        name: "Second Oslo Bot",
        exact: true,
      });
      await secondAgentItem.waitFor({ state: "visible" });
      await secondAgentItem.click();
      await page.waitForURL(/\/dashboard\/machines\/runtime_second-oslo-bot$/u);

      const secondMachineSwitcher = page.getByRole("button", {
        name: "Second Oslo Bot, Online",
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
      await page.getByLabel("Launch Code", { exact: true }).waitFor({ state: "visible" });
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
      await page.getByLabel("Launch Code", { exact: true }).waitFor({ state: "visible" });
      core.state.creationError = "Launch Code is invalid or expired.";
      await prepareAgentLaunch(page, "Second Oslo Bot", "invalid-code");
      await page.getByRole("button", { name: "Launch agent", exact: true }).click();
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
        .getByRole("alert")
        .filter({ hasText: /^Launch Code is invalid or expired\.$/u })
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
        `http://127.0.0.1:${dashboardPort}/dashboard?creation=agent_request_second`
      );
      await page
        .getByRole("heading", { name: "Second Oslo Bot is alive!" })
        .waitFor({ state: "visible" });
      assert.match(
        page.url(),
        /\/dashboard\?creation=agent_request_second$/u,
        "a ready agent should pause on the Ready interstitial"
      );
      await page.locator(".status-prism-scene--happy").waitFor({ state: "visible" });
      await onboardingScreenshots(page, "ready");
      const meetAgent = page.getByRole("link", { name: "Continue to chat" });
      assert.equal(
        await meetAgent.getAttribute("href"),
        "/dashboard/machines/runtime_second-oslo-bot/chat"
      );
      await meetAgent.click();
      await page.waitForURL(
        /\/dashboard\/machines\/runtime_second-oslo-bot\/chat$/u
      );
      await page
        .getByRole("navigation", { name: "Agent, topics, and chats" })
        .waitFor({ state: "visible" });
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

      const creationPostCountBeforeChat = core.state.creationPosts.length;
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
      assert.equal(core.state.creationPosts.length, creationPostCountBeforeChat, "chat recovery created another Project");
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
      const openRouterSignIn = page.getByRole("link", { name: "Sign in with OpenRouter" });
      assert.match(
        (await openRouterSignIn.getAttribute("href")) ?? "",
        /^\/openrouter\/start\?machineId=/u,
        "OpenRouter sign-in should route through the dashboard OAuth start endpoint"
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
      const connectionsPath =
        "/dashboard/machines/runtime_completed-oslo-bot/connections";
      const connectionsSidebar = page.getByRole("navigation", {
        name: "Agent, topics, and chats",
      });
      const generalChatOnConnections = connectionsSidebar.getByRole("button", {
        name: "General",
        exact: true,
      });
      await generalChatOnConnections.hover();
      await connectionsSidebar
        .getByRole("button", { name: "Rename General", exact: true })
        .click();
      const sidebarRenameDialog = page.getByRole("dialog", {
        name: "Rename chat",
      });
      await sidebarRenameDialog
        .getByRole("textbox", { name: "Name" })
        .fill("Sidebar QA");
      await sidebarRenameDialog.getByRole("button", { name: "Save" }).click();
      await connectionsSidebar
        .getByRole("button", { name: "Sidebar QA", exact: true })
        .waitFor({ state: "visible" });
      assert.equal(
        new URL(page.url()).pathname,
        connectionsPath,
        "renaming a sidebar chat must preserve the current non-chat route"
      );

      await page.getByRole("main").evaluate((element) => {
        element.scrollTop = 120;
      });
      const completedProductNav = page.getByRole("navigation", {
        name: "Agent navigation",
      });
      const completedBrainLink = completedProductNav.getByRole("link", {
        name: "Brain", exact: true,
      });
      await completedBrainLink.waitFor({ state: "visible" });
      assert.equal(
        await completedBrainLink.getAttribute("href"),
        "/dashboard/machines/runtime_completed-oslo-bot/brain"
      );
      await page.goto(
        `http://127.0.0.1:${dashboardPort}/dashboard/machines/completed-oslo-bot/chat`
      );

      const chatSidebar = page.getByRole("navigation", {
        name: "Agent, topics, and chats",
      });
      await chatSidebar
        .getByRole("button", { name: "New chat in General", exact: true })
        .waitFor({ state: "visible" });
      await chatSidebar
        .getByRole("button", { name: "Collapse General", exact: true })
        .click();
      await chatSidebar
        .getByRole("button", { name: "Sidebar QA", exact: true })
        .waitFor({ state: "hidden" });
      await chatSidebar
        .getByRole("button", { name: "Expand General", exact: true })
        .click();
      await chatSidebar
        .getByRole("button", { name: "Sidebar QA", exact: true })
        .waitFor({ state: "visible" });
      await chatSidebar
        .getByRole("button", { name: "Archive Sidebar QA", exact: true })
        .click();
      await waitFor(() =>
        hostedDevice.state.actions.some((action) => {
          const payload = action.SetChatArchived as
            | { archived?: unknown }
            | undefined;
          return actionName(action) === "SetChatArchived" && payload?.archived === true;
        })
      );
      await chatSidebar
        .getByRole("button", { name: "Restore Sidebar QA", exact: true })
        .waitFor({ state: "visible" });
      await page
        .locator(".finite-chat__topbar")
        .getByText("Sidebar QA", { exact: true })
        .waitFor({ state: "visible" });

      await page.reload();
      const restoredChatRow = chatSidebar
        .locator(".finite-chat__thread-row")
        .filter({ hasText: "Sidebar QA" });
      await restoredChatRow.hover();
      const restoreChatButton = restoredChatRow.getByRole("button", {
        name: "Restore Sidebar QA",
        exact: true,
      });
      await restoreChatButton.waitFor({ state: "visible" });
      const archiveActionCount = hostedDevice.state.actions.filter(
        (action) => actionName(action) === "SetChatArchived"
      ).length;
      await restoreChatButton.click();
      await waitFor(
        () =>
          hostedDevice.state.actions.filter(
            (action) => actionName(action) === "SetChatArchived"
          ).length === archiveActionCount + 1
      );
      await chatSidebar
        .getByRole("button", { name: "Archive Sidebar QA", exact: true })
        .waitFor({ state: "visible" });

      await page.getByRole("button", { name: "Rename chat" }).click();
      const renameDialog = page.getByRole("dialog", { name: "Rename chat" });
      await renameDialog.getByRole("textbox", { name: "Name" }).fill("Browser QA");
      await renameDialog.getByRole("button", { name: "Save" }).click();
      await renameDialog.waitFor({ state: "hidden" });
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
          topic_id: "home",
          chat_id: "chat_browser_agent",
          text: message,
          metadata_json: null,
        },
      });
      assert.equal(
        hostedDevice.state.app.messages.some(
          (candidate) => candidate.is_mine && candidate.text === message
        ),
        true
      );

      hostedDevice.setUpdatesAvailable(false);
      await page.waitForTimeout(500);
      assert.equal(
        await page.getByText("Chat needs attention", { exact: true }).count(),
        0,
        "a transient update-stream reconnect rendered an action-required alert"
      );
      assert.equal(
        await page.getByText("Reconnecting", { exact: true }).count(),
        0,
        "the reconnect notice flashed before the grace period elapsed"
      );
      await page
        .getByText("Reconnecting", { exact: true })
        .waitFor({ state: "visible", timeout: 5_000 });
      assert.equal(
        await page.getByText("Chat needs attention", { exact: true }).count(),
        0,
        "a prolonged update-stream reconnect rendered an action-required alert"
      );
      hostedDevice.setUpdatesAvailable(true);
      await page
        .getByText("Reconnecting", { exact: true })
        .waitFor({ state: "hidden", timeout: 5_000 });

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
      await page.getByRole("button", { name: "Start audio recording" }).click();
      await page.getByRole("status").filter({ hasText: "Recording 0:00 / 10:00" }).waitFor({
        state: "visible",
      });
      await page.waitForTimeout(300);
      await page.getByRole("button", { name: "Stop audio recording" }).click();
      const recordedAttachment = page
        .locator(".finite-chat__attachment-chip")
        .filter({ hasText: "voice-" });
      await recordedAttachment.waitFor({ state: "visible", timeout: 15_000 });
      assert.match(await recordedAttachment.innerText(), /\.webm$/u);
      await recordedAttachment.getByRole("button").click();
      await recordedAttachment.waitFor({ state: "hidden" });

      await page.locator('input[type="file"]').setInputFiles({
        name: "browser-proof.png",
        mimeType: "image/png",
        buffer: PNG_BYTES,
      });
      await page.getByLabel("Message your agent").fill("Image from browser.");
      await page.getByRole("button", { name: "Send message" }).click();
      await page.getByRole("img", { name: "browser-proof.png" }).waitFor({ state: "visible" });

      const agentAttachmentPath =
        "/hosted-device/attachments/room_browser_agent/message_4/attachment_4";
      hostedDevice.state.app.messages.push(
        hostedImageMessage("Image returned by agent.", false, 4, "agent-proof.png")
      );
      hostedDevice.emit();
      const agentImage = page.getByRole("img", { name: "agent-proof.png" });
      await agentImage.waitFor({ state: "visible" });
      await agentImage.evaluate((image) => image.scrollIntoView({ block: "center" }));
      const agentAttachmentHref = await agentImage.getAttribute("src");
      assert(
        agentAttachmentHref && agentAttachmentHref.includes(agentAttachmentPath),
        `agent attachment image did not use the authenticated route: ${agentAttachmentHref}`
      );
      // Verify the authenticated attachment route explicitly from the signed-in
      // browser page instead of racing the image element's own request.
      const attachmentResponse = await page.evaluate(async (href) => {
        const response = await fetch(href, { cache: "no-store" });
        const bytes = await response.arrayBuffer();
        return {
          status: response.status,
          url: response.url,
          contentType: response.headers.get("content-type") ?? "",
          byteLength: bytes.byteLength,
        };
      }, agentAttachmentHref);
      assert.equal(
        attachmentResponse.status,
        200,
        `agent attachment route returned ${attachmentResponse.status}`
      );
      assert.match(
        attachmentResponse.contentType,
        /^image\/png\b/u,
        "agent attachment route did not return PNG bytes"
      );
      assert(attachmentResponse.byteLength > 0, "agent attachment route returned an empty body");
      await waitFor(
        () =>
          agentImage.evaluate(
            (image) =>
              image instanceof HTMLImageElement && image.complete && image.naturalWidth > 0
          ),
        15_000,
        async () =>
          `agent attachment returned 200 but did not decode\nURL: ${attachmentResponse.url}\n${await pageText(page)}`
      );
      assert.equal(
        hostedDevice.state.authRequests.some((request) =>
          request.path.startsWith("/v1/app/attachments/")
        ),
        true,
        "attachment bytes must traverse the authenticated hosted-device route"
      );

      const directImageUrl = `${hostedDevice.url}/generated-image.png`;
      hostedDevice.state.app.messages.push(
        hostedRemoteImageMessage(
          "Image generated by Hermes.",
          5,
          "hermes-generated.png",
          directImageUrl
        )
      );
      hostedDevice.emit();
      const loadDirectImage = page.getByRole("button", {
        name: "Load hermes-generated.png",
      });
      await loadDirectImage.waitFor({ state: "visible" });
      assert.equal(
        await page.getByRole("img", { name: "hermes-generated.png" }).count(),
        0,
        "a remote image must not be placed in the document before user consent"
      );
      assert.equal(
        hostedDevice.state.directImageGets,
        0,
        "a URL-only Hermes image must not make a remote request before user consent"
      );

      await loadDirectImage.click();
      const directImage = page.getByRole("img", { name: "hermes-generated.png" });
      await directImage.waitFor({ state: "visible" });
      assert.equal(
        await directImage.getAttribute("src"),
        directImageUrl,
        "a URL-only Hermes image must load from its remote URL after user consent"
      );
      await waitFor(
        () =>
          directImage.evaluate(
            (image) =>
              image instanceof HTMLImageElement && image.complete && image.naturalWidth > 0
          ),
        15_000,
        async () =>
          `URL-only Hermes image did not decode\n${await pageText(page)}`
      );
      assert.equal(hostedDevice.state.directImageGets, 1);

      hostedDevice.state.app.messages.push(
        hostedPlayableMessage("Video returned by agent.", 6, "agent-proof.mp4", "Video", "video/mp4")
      );
      hostedDevice.state.app.messages.push(
        hostedPlayableMessage("Audio returned by agent.", 7, "agent-proof.mp3", "VoiceNote", "audio/mpeg")
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
          topic_id: "home",
          chat_id: "chat_browser_agent",
          account_id: "agent-account-browser",
          device_id: "agent",
          display_name: "Completed Oslo Bot",
          activity_kind: "working",
        },
      ];
      hostedDevice.emit();
      await expectVisibleText(page, "Completed Oslo Bot is working");

      const browserQaTool: FakeHostedChatState["messages"][number] = {
        ...hostedMessage("💻 Running browser QA", false, 7),
        kind: "tool",
        status: "complete",
      };
      hostedDevice.state.app.messages.push(browserQaTool);
      hostedDevice.emit();
      await expectVisibleText(page, "Working · 1 step");
      const browserQaRollup = page
        .locator("details.finite-chat__tool-rollup")
        .filter({ hasText: "Running browser QA" });
      assert.equal(
        await browserQaRollup.evaluate((element) => (element as HTMLDetailsElement).open),
        true,
        "a newly observed active tool rollup should open once"
      );
      await browserQaRollup.locator("summary").click();
      assert.equal(
        await browserQaRollup.evaluate((element) => (element as HTMLDetailsElement).open),
        false,
        "the user should be able to close an active tool rollup"
      );

      browserQaTool.text = "💻 Running browser QA · refreshed";
      browserQaTool.display_content = "💻 Running browser QA · refreshed";
      hostedDevice.emit();
      await waitFor(
        async () =>
          (await browserQaRollup.locator("pre").textContent())
          === "💻 Running browser QA · refreshed",
        15_000,
        () => "the streamed tool edit did not reach the closed rollup"
      );
      assert.equal(
        await browserQaRollup.evaluate((element) => (element as HTMLDetailsElement).open),
        false,
        "a message edit and stream rerender must preserve the user's closed state"
      );

      hostedDevice.state.app.typing_members = [];
      hostedDevice.state.app.messages.push({
        ...hostedMessage("Browser QA complete.", false, 8),
        final_delivery: true,
      });
      hostedDevice.emit();
      await expectVisibleText(page, "Worked through 1 step");
      const completedResponse = page
        .locator(".finite-chat__message--agent")
        .filter({ hasText: "Browser QA complete." });
      await completedResponse
        .getByRole("button", { name: "Copy response" })
        .waitFor({ state: "visible" });
      await completedResponse
        .getByRole("button", { name: "Share response" })
        .waitFor({ state: "visible" });
      assert.equal(
        await completedResponse.locator(".finite-chat__message-actions time").count(),
        1,
        "a completed response timestamp must share the response-action row"
      );
      assert.equal(
        await page
          .locator(".finite-chat__message--agent")
          .filter({ hasText: "Video returned by agent." })
          .getByRole("button", { name: "Copy response" })
          .count(),
        0,
        "non-final agent messages must not offer response actions"
      );
      assert.equal(
        await browserQaRollup.evaluate((element) => (element as HTMLDetailsElement).open),
        false,
        "completion must not reopen a rollup the user closed"
      );
      await page
        .getByText("Completed Oslo Bot is working", { exact: true })
        .waitFor({ state: "hidden", timeout: 15_000 });

      await browserQaRollup.locator("summary").click();
      assert.equal(
        await browserQaRollup.evaluate((element) => (element as HTMLDetailsElement).open),
        true,
        "the user should be able to reopen a completed tool rollup"
      );
      await page.getByLabel("Message your agent").fill("Working lease browser proof.");
      await page.getByRole("button", { name: "Send message" }).click();
      await page
        .getByRole("article")
        .getByText("Working lease browser proof.", { exact: true })
        .waitFor({ state: "visible", timeout: 15_000 });
      assert.equal(
        await browserQaRollup.evaluate((element) => (element as HTMLDetailsElement).open),
        true,
        "unrelated chat activity must preserve the user's open state"
      );
      hostedDevice.state.app.typing_members = [
        {
          room_id: "room_browser_agent",
          topic_id: "home",
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
        assert.equal(exchange.serviceAuthorization, `Bearer ${SITES_VIEWER_SESSION_TOKEN}`);
        assert.equal(exchange.outputUrl, localSiteUrl);
        assert.equal(exchange.verifiedEmail, "browser@finite.vip");
        assert.equal(exchange.returnTo, "/");
      }
      assert.equal(sites.state.redemptions, 1);
      assert(sites.state.privateContentRequests >= 1);
      assert.match(
        (await page.getByLabel("Site preview").locator("iframe").getAttribute("src")) ?? "",
        /\/_finite\/auth\?session_token=/u
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
          archived: false,
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
          topic_id: "home",
          chat_id: "chat_browser_agent",
          text: canonicalReply,
          metadata_json: null,
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
        topic_id: "home",
        reason: null,
        intent_key: hostedDevice.state.newChatRequests[0]!.intent_key,
      });
      assert.match(String(hostedDevice.state.newChatRequests[0]!.intent_key), /.+/);
      assert.equal(hostedDevice.state.app.selected_room_id, "room_browser_agent");
      assert.equal(hostedDevice.state.app.selected_topic_id, "home");
      assert(
        hostedDevice.state.app.topics
          .find((topic) => topic.room_id === "room_browser_agent")
          ?.chats.some((chat) => chat.chat_id === "chat_browser_new_1")
      );

      // Creating a topic also changes the selected Room/Topic/Chat, so it must
      // share the ordered navigation lane with a subsequent chat click.
      const topicTitle = "Launch planning";
      const topicActionsBefore = hostedDevice.state.actions.length;
      const topicNavigationActions = () => hostedDevice.state.actions
        .slice(topicActionsBefore)
        .filter((action) => ["CreateTopic", "OpenChat"].includes(actionName(action)));
      // Typing is independent of the ordered navigation lane. Include it
      // explicitly so this proof cannot rely on incidental request order.
      const typing = await page.request.post(
        new URL("/api/chat/machines/completed-oslo-bot/hosted-device/actions", page.url()).href,
        { data: { SetTyping: { room_id: "room_browser_agent", is_typing: false } } }
      );
      assert(typing.ok());
      const completedTopicMutations = hostedDevice.state.completedSelectionMutations;
      hostedDevice.holdNextNavigationAction();
      await page.getByRole("button", { name: "New topic", exact: true }).click();
      const newTopicDialog = page.getByRole("dialog", { name: "New topic" });
      await newTopicDialog.getByLabel("Name").fill(topicTitle);
      await newTopicDialog.getByRole("button", { name: "Create topic" }).click();
      await waitFor(
        () => hostedDevice.state.navigationActionGate === null,
        5_000,
        () => "CreateTopic did not enter the ordered navigation lane"
      );
      assert.deepEqual(topicNavigationActions(), [{
        CreateTopic: {
          room_id: "room_browser_agent",
          title: topicTitle,
        },
      }]);

      // The modal intentionally blocks a second human click. Trigger the
      // underlying existing-chat handler directly to prove the provider still
      // serializes the two selection-changing mutations if they overlap.
      await page
        .locator(".finite-chat__folder-body")
        .locator("button")
        .filter({ hasText: "Browser QA" })
        .first()
        .evaluate((element) => (element as HTMLButtonElement).click());
      await waitFor(
        () => topicNavigationActions().length > 1,
        750
      ).catch(() => undefined);
      assert.equal(
        topicNavigationActions().length,
        1,
        "a later OpenChat reached the daemon before CreateTopic completed"
      );
      hostedDevice.releaseNavigationAction();
      await waitFor(
        () => hostedDevice.state.completedSelectionMutations === completedTopicMutations + 2,
        5_000,
        () => "CreateTopic and the subsequent OpenChat did not both finish"
      );
      assert(
        hostedDevice.state.app.topics.some(
          (topic) => topic.room_id === "room_browser_agent" && topic.title === topicTitle
        )
      );
      assert.equal(
        hostedDevice.state.app.selected_chat_id,
        "chat_browser_agent",
        "the delayed CreateTopic response overrode the user's later chat selection"
      );
      await newTopicDialog.waitFor({ state: "hidden" });
      await page
        .locator(".finite-chat__topbar")
        .getByText("Browser QA", { exact: true })
        .waitFor({ state: "visible" });

      hostedDevice.state.app.messages.push({
        ...hostedMessage("Remembered transcript only.", false, 13),
        message_id: "message_remembered_only",
        chat_id: "chat_browser_remembered",
      });
      const completedBeforeRememberedNavigation =
        hostedDevice.state.completedSelectionMutations;
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
        () =>
          hostedDevice.state.completedSelectionMutations
            === completedBeforeRememberedNavigation + 1,
        5_000,
        () => "the daemon never persisted the Remembered work selection"
      );

      // Hold a selection-only OpenChat while a newer stream revision lands.
      // Starting outside chat also proves route navigation does not wait for
      // the high-latency mutation response. The clicked selection is pinned
      // client-side immediately, so the pane switches at once. The scoped
      // stream and HTTP read must return that transcript even before the
      // shared Device cursor catches up.
      const stateFetchesBeforeSelectionRace = hostedDevice.state.authRequests.filter(
        (request) => request.path === "/v1/app/state"
      ).length;
      await page
        .getByRole("navigation", { name: "Agent navigation" })
        .getByRole("link", { name: "Connections", exact: true })
        .click();
      await page.waitForURL(/\/connections$/u);
      const completedBeforeSelectionRace =
        hostedDevice.state.completedSelectionMutations;
      hostedDevice.holdNextNavigationAction();
      await page
        .locator(".finite-chat__folder-body")
        .getByRole("button", { name: "Browser QA", exact: true })
        .click();
      await page.waitForURL(/\/chat$/u);
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
      // While OpenChat is held, the stream must already serve Browser QA's
      // transcript instead of the shared cursor's Remembered work transcript.
      await expectVisibleText(page, "Concurrent stream update.");
      hostedDevice.releaseNavigationAction();
      await waitFor(
        () =>
          hostedDevice.state.completedSelectionMutations
            === completedBeforeSelectionRace + 1,
        5_000,
        () => "the daemon never persisted the Browser QA selection"
      );
      await waitFor(
        () =>
          hostedDevice.state.authRequests.filter(
            (request) => request.path === "/v1/app/state"
          ).length > stateFetchesBeforeSelectionRace,
        5_000,
        () => "navigation did not refresh its scoped transcript"
      );
      await page
        .locator(".finite-chat__topbar")
        .getByText("Browser QA", { exact: true })
        .waitFor({ state: "visible" });
      assert.equal(hostedDevice.state.app.selected_chat_id, "chat_browser_agent");
      hostedDevice.state.app.messages.push({
        ...hostedMessage("Selection settled checkpoint.", false, 15),
        message_id: "message_selection_settled",
        chat_id: "chat_browser_agent",
      });
      hostedDevice.emit();
      await expectVisibleText(page, "Selection settled checkpoint.");

      // Another tab's selection must not move this tab or erase its history.
      // Check actual foreground content as well as the selected heading.
      hostedDevice.state.app.selected_chat_id = "chat_browser_remembered";
      hostedDevice.state.app.topics[0]!.active_chat_id = "chat_browser_remembered";
      hostedDevice.state.app.messages.push({
        ...hostedMessage("Remembered background stream.", false, 16),
        message_id: "message_remembered_background",
        chat_id: "chat_browser_remembered",
      });
      hostedDevice.state.app.rev += 1;
      hostedDevice.emit();
      await page
        .locator(".finite-chat__topbar")
        .getByText("Browser QA", { exact: true })
        .waitFor({ state: "visible" });
      assert.equal(
        await page
          .locator(".finite-chat__message")
          .getByText("Remembered background stream.", { exact: true })
          .isVisible(),
        false,
        "a divergent daemon selection moved the visible Chat without a local click"
      );
      await expectVisibleText(page, "Selection settled checkpoint.");

      const completedBeforeBackgroundChatNavigation =
        hostedDevice.state.completedSelectionMutations;
      await page
        .getByRole("button", { name: "Remembered work", exact: true })
        .click();
      await page
        .locator(".finite-chat__topbar")
        .getByText("Remembered work", { exact: true })
        .waitFor({ state: "visible" });
      await expectVisibleText(page, "Remembered background stream.");
      // The browser-owned selection presents instantly; let the daemon confirm before
      // reading mutation counters so the next section starts quiescent.
      await waitFor(
        () =>
          hostedDevice.state.completedSelectionMutations
            === completedBeforeBackgroundChatNavigation + 1,
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
        await page.getByRole("button", { name: "Retire agent" }).count(),
        0,
        "ordinary destroy must stay hidden without an explicit Runtime Retirement capability"
      );
      assert.deepEqual(core.state.destroyPosts, []);
    });
  } finally {
    await browser?.close().catch(() => {});
    await Promise.all([
      stopChildProcess(dashboard),
      stopChildProcess(paidDashboard),
    ]);
    core.server.close();
    hostedDevice.close();
    sites.server.close();
    await resetDashboardDevDirs();
  }
});

async function resetDashboardDevDirs() {
  await Promise.all([
    rm(".next-browser-test", { recursive: true, force: true }),
    rm(".next-browser-stripe-test", { recursive: true, force: true }),
  ]);
}

async function stopChildProcess(process: ChildProcessWithoutNullStreams | null) {
  if (!process) return;
  if (process.exitCode !== null || process.signalCode !== null) return;
  const exited = once(process, "exit");
  process.kill("SIGTERM");
  const terminated = await Promise.race([
    exited.then(() => true),
    new Promise<false>((resolve) => setTimeout(() => resolve(false), 2_000)),
  ]);
  if (terminated) return;
  process.kill("SIGKILL");
  await Promise.race([
    exited,
    new Promise((resolve) => setTimeout(resolve, 2_000)),
  ]);
}

function startDashboard(
  port: number,
  coreUrl: string,
  hostedDeviceUrl: string,
  sitesUrl: string,
  options: {
    admin?: boolean;
    stripeConfigured?: boolean;
    runtimeRetirement?: boolean;
    distDir?: string;
  } = {}
) {
  const adminOrganizationId = "org_browser_admin";
  const devAccessToken = options.admin
    ? `fixture.${Buffer.from(
        JSON.stringify({ org_id: adminOrganizationId })
      ).toString("base64url")}.signature`
    : "fixture-browser-access-token";
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
        FC_SITES_UPSTREAM_URL: sitesUrl,
        FINITE_SITES_VIEWER_SESSION_TOKEN: SITES_VIEWER_SESSION_TOKEN,
        FC_SITES_ALLOW_LOCAL_OUTPUTS: "1",
        FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
        FC_DASHBOARD_DEV_EMAIL: "browser@finite.vip",
        FC_DASHBOARD_DEV_WORKOS_USER_ID: "user_browser",
        FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: devAccessToken,
        FC_WORKOS_OPERATOR_ORG_ID: options.admin ? adminOrganizationId : "",
        FC_DASHBOARD_RUNTIME_MODE: options.stripeConfigured ? "customer" : "canary",
        FC_DASHBOARD_ENABLE_RUNTIME_RETIREMENT: options.runtimeRetirement ? "1" : "",
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
    if (
      request.method === "POST"
      && requestUrl.pathname === "/internal/v1/viewer-sessions"
    ) {
      const body = (await readJson(request)) as Record<string, unknown>;
      const exchange = {
        serviceAuthorization: singleHeader(request.headers.authorization),
        outputUrl: String(body.output_url ?? body.site_url ?? ""),
        verifiedEmail: String(body.verified_email ?? ""),
        returnTo: String(body.return_to ?? ""),
      };
      state.exchanges.push(exchange);
      if (
        exchange.serviceAuthorization !== `Bearer ${SITES_VIEWER_SESSION_TOKEN}`
        || exchange.outputUrl !== siteUrl
        || exchange.verifiedEmail !== "browser@finite.vip"
        || exchange.returnTo !== "/"
      ) {
        writeJson(response, 403, { error: "viewer access unavailable" });
        return;
      }
      writeJson(response, 200, {
        redeem_url: `${siteUrl}_finite/auth?session_token=${token}&return_to=%2F`,
      });
      return;
    }

    if (request.method === "GET" && requestUrl.pathname === "/_finite/auth") {
      if (requestUrl.searchParams.get("session_token") !== token) {
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

async function startFakeHostedDevice() {
  const app = initialHostedChatState();
  const state: HostedDeviceState = {
    unavailable: false,
    updatesUnavailable: false,
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
    directImageGets: 0,
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
  const streams = new Map<ServerResponse, URLSearchParams>();
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
        for (const stream of streams.keys()) stream.end();
        streams.clear();
      }
    },
    setUpdatesAvailable(available: boolean) {
      state.updatesUnavailable = !available;
      if (!available) {
        for (const stream of streams.keys()) stream.end();
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
      for (const stream of streams.keys()) {
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
  streams: Map<ServerResponse, URLSearchParams>
) {
  const url = new URL(request.url ?? "/", "http://fixture");
  const path = url.pathname;
  const view = url.searchParams;
  if (request.method === "GET" && path === "/generated-image.png") {
    state.directImageGets += 1;
    response.writeHead(200, {
      "cache-control": "public, max-age=3600",
      "content-length": String(PNG_BYTES.length),
      "content-type": "image/png",
    });
    response.end(PNG_BYTES);
    return;
  }
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
    writeJson(response, 200, hostedView(state.app, view));
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
        writeJson(response, 503, {
          error:
            "canonical Agent conversation requires recovery: first-time binding bootstrap was not authorized by Project creation",
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
    writeJson(response, 200, hostedView(state.app, view));
    return;
  }

  if (request.method === "POST" && path === "/v1/app/attachments") {
    await readBytes(request);
    state.app.messages.push(
      hostedImageMessage("Image sent from browser.", true, state.app.messages.length + 1, "browser-proof.png")
    );
    state.app.rev += 1;
    emitHostedState(streams, state.app);
    writeJson(response, 200, hostedView(state.app, view));
    return;
  }

  if (request.method === "POST" && path === "/v1/app/new-chat") {
    const body = (await readJson(request)) as Record<string, unknown>;
    state.newChatRequests.push(body);
    assert.equal(body.project_id, "project_running");
    assert.equal(body.room_id, "room_browser_agent");
    assert.equal(body.topic_id, "home");
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
    writeJson(response, 200, hostedView(state.app, view));
    return;
  }

  if (request.method === "GET" && path === "/v1/app/state") {
    writeJson(response, 200, hostedView(state.app, view));
    return;
  }

  if (request.method === "POST" && path === "/v1/app/actions") {
    const action = (await readJson(request)) as Record<string, unknown>;
    const navigationAction = ["OpenRoom", "OpenTopic", "OpenChat", "CreateTopic"].includes(
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
    writeJson(response, 200, hostedView(state.app, view));
    return;
  }

  if (request.method === "GET" && path === "/v1/app/updates") {
    if (state.updatesUnavailable) {
      writeJson(response, 503, { error: "hosted chat updates are temporarily unavailable" });
      return;
    }
    response.writeHead(200, {
      "cache-control": "no-cache",
      connection: "keep-alive",
      "content-type": "text/event-stream",
    });
    streams.set(response, view);
    response.on("close", () => streams.delete(response));
    writeHostedState(response, state.app, view);
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
        model: String(body.model ?? "deepseek/deepseek-v4-flash-0731"),
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
          topic_id: "home",
          title: "General",
          active_chat_id: "chat_browser_agent",
          chats: [
            {
              chat_id: "chat_browser_agent",
              title: "General",
              active: true,
              archived: false,
            },
            {
              chat_id: "chat_browser_remembered",
              title: "Remembered work",
              active: false,
              archived: false,
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
      state.selected_topic_id = "home";
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
  } else if (operation === "CreateTopic") {
    const payload = action.CreateTopic as Record<string, unknown> | undefined;
    assert(payload);
    const roomId = String(payload.room_id ?? "");
    const title = String(payload.title ?? "");
    assert(title && state.rooms.some((room) => room.room_id === roomId));
    const createdCount = state.topics.filter(
      (topic) => topic.room_id === roomId && topic.topic_id.startsWith("topic_browser_created_")
    ).length;
    const topicId = `topic_browser_created_${createdCount + 1}`;
    const chatId = `chat_browser_created_${createdCount + 1}`;
    state.topics.push({
      room_id: roomId,
      topic_id: topicId,
      title,
      active_chat_id: chatId,
      chats: [{ chat_id: chatId, title: "New chat", active: true, archived: false }],
    });
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
    topic.chats.push({ chat_id: chatId, title: "New chat", active: true, archived: false });
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
  } else if (operation === "SetChatArchived") {
    const payload = action.SetChatArchived as Record<string, unknown> | undefined;
    assert(payload && typeof payload.archived === "boolean");
    const topic = state.topics.find(
      (candidate) =>
        candidate.room_id === payload.room_id
        && candidate.topic_id === payload.topic_id
    );
    const chat = topic?.chats.find((candidate) => candidate.chat_id === payload.chat_id);
    assert(chat);
    chat.archived = payload.archived;
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
    conversation_id: "home",
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

function hostedRemoteImageMessage(
  text: string,
  seq: number,
  filename: string,
  url: string
): FakeHostedChatState["messages"][number] {
  return {
    ...hostedMessage(text, false, seq),
    kind: "media",
    media: [
      {
        attachment_id: url,
        url,
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
  streams: Map<ServerResponse, URLSearchParams>,
  state: FakeHostedChatState
) {
  for (const [stream, view] of streams) {
    writeHostedState(stream, state, view);
  }
}

function hostedView(state: FakeHostedChatState, view: URLSearchParams) {
  const room = view.get("room_id") ?? state.selected_room_id;
  const topicId = view.get("topic_id") ?? state.selected_topic_id;
  const topic = state.topics.find(topic => topic.room_id === room && topic.topic_id === topicId);
  const chat = view.get("chat_id") ?? (view.has("room_id") ? topic?.active_chat_id ?? null : state.selected_chat_id);
  const messages = state.messages.filter(message => message.room_id === room && message.conversation_id === topicId && message.chat_id === chat);
  const anchorIndex = messages.findIndex(message => message.message_id === view.get("oldest_message_id"));
  const limit = Math.max(Number(view.get("limit") ?? 50), anchorIndex < 0 ? 0 : messages.length - anchorIndex);
  return { ...state, selected_room_id: room, selected_topic_id: topicId, selected_chat_id: chat,
    messages: messages.slice(-limit),
  };
}

function writeHostedState(response: ServerResponse, state: FakeHostedChatState, view: URLSearchParams) {
  response.write(`id: ${state.rev}\nevent: state\ndata: ${JSON.stringify(hostedView(state, view))}\n\n`);
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
  const authorization = request.headers.authorization;
  if (
    authorization !== `Bearer ${CORE_TOKEN}` &&
    authorization !== "Bearer fixture-browser-access-token" &&
    !authorization?.startsWith("Bearer fixture.")
  ) {
    writeJson(response, 401, { error: "missing service token" });
    return;
  }

  if (request.method === "GET" && request.url === "/api/core/v1/me") {
    state.meGets += 1;
    if (state.meError) {
      writeJson(response, 503, { error: "Core account is unavailable for browser proof." });
      return;
    }
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
    if (state.billingError) {
      writeJson(response, 503, { error: "Billing is temporarily unavailable for browser proof." });
      return;
    }
    writeJson(response, 200, {
      customer_org: {
        id: "org_browser",
        owner_user_id: "user_browser",
        name: "Browser Test",
        billing_class: state.billingClass,
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
    page.setDefaultNavigationTimeout(90_000);
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
    meError: false,
    runtimeRouteGets: 0,
    runtimeRouteProjectIdOverride: null,
    cancelPosts: [],
    destroyPosts: [],
    recoverPosts: [],
    restartPosts: [],
    createDelayMs: 0,
    canCreateAgent: false,
    requiresBilling: true,
    billingClass: "sponsored",
    billingError: false,
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
  isRelocation = false,
}: {
  id: string;
  projectId: string;
  displayName?: string;
  status: AgentCreationRequest["status"];
  failureMessage?: string | null;
  createdAt?: string;
  agentRuntimeId?: string | null;
  isRelocation?: boolean;
}): AgentCreationRequest {
  return {
    id,
    project_id: projectId,
    display_name: displayName,
    profile_picture_url: null,
    is_relocation: isRelocation,
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
    const response = await fetch(`http://127.0.0.1:${port}/favicon.svg`, {
      redirect: "manual",
      signal: AbortSignal.timeout(5_000),
    }).catch(() => null);
    return Boolean(response && response.status < 500);
  }, 60_000, () => `dashboard did not become ready\n${output()}`);
}

async function expectVisibleText(page: Page, text: string) {
  // Next.js copies the page's h1 into its persistent, visually-hidden route
  // announcer (#__next-route-announcer__) after client-side transitions, so an
  // unscoped text match can resolve to two mounted elements and trip strict
  // mode depending on announcer timing. The announcer is never page content;
  // exclude it.
  await page
    .getByText(text, { exact: true })
    .and(page.locator(":not(#__next-route-announcer__)"))
    .waitFor({ state: "visible", timeout: 15_000 });
}

async function onboardingScreenshots(page: Page, stage: string) {
  const directory = process.env.ONBOARDING_SCREENSHOT_DIR;
  if (!directory) return;
  await mkdir(directory, { recursive: true });
  const original = page.viewportSize();
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.screenshot({ path: `${directory}/${stage}-${width}.png`, fullPage: true });
    assert(await page.evaluate(() => {
      const main = document.querySelector("main");
      const heading = main?.querySelector("h1")?.getBoundingClientRect();
      return Boolean(main && main.scrollWidth <= main.clientWidth && heading &&
        heading.left >= 0 && heading.right <= window.innerWidth);
    }), `${stage} is clipped or overflows horizontally at ${width}px`);
  }
  if (original) await page.setViewportSize(original);
}

async function prepareAgentLaunch(page: Page, displayName: string, code = "fixture-launch-code") {
  const codeInput = page.getByLabel("Launch Code", { exact: true });
  await codeInput.waitFor({ state: "visible", timeout: 15_000 });
  await codeInput.fill(code);
  await page.getByRole("button", { name: "Continue with code", exact: true }).click();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page.getByLabel("Agent name").fill(displayName);
  await waitFor(async () => page.getByRole("button", { name: "Launch agent", exact: true }).isEnabled());
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
