import { spawn } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { once } from "node:events";
import fs from "node:fs";
import http, { type IncomingMessage, type ServerResponse } from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const MACHINE_ID = "web-design-fixture";
const RUNTIME_ID = "runtime_web_design";
const CORE_TOKEN = "web-design-core-token";
const DEVICE_TOKEN = "web-design-hosted-device-token";
const WORKOS_USER_ID = "user_web_design";
const FIXTURE_EMAIL = "fixture-user@example.test";
const DEFAULT_PORT = 13002;
const VALID_SCENARIOS = new Set(["healthy", "unavailable", "recovering"]);

type Scenario = "healthy" | "unavailable" | "recovering";

type FixtureState = {
  rev: number;
  chatArchived: boolean;
  chatTitle: string;
  newChatIntents?: Record<string, string>;
  selectedNewChatId?: string | null;
  extraTopics: Array<{
    topicId: string;
    title: string;
    createdSeq: number;
  }>;
  messages: Array<Record<string, unknown>>;
};

type FixtureAttachment = {
  bytes: Buffer;
  filename: string;
  mimeType: string;
};

const dashboardDir = process.cwd();
const repoRoot = path.resolve(dashboardDir, "../../..");
const stateDir = path.join(repoRoot, ".local-state", "web-design-fixture");
const statePath = path.join(stateDir, "chat.json");
const attachmentsDir = path.join(stateDir, "attachments");
const scenarioPath = path.join(stateDir, "scenario");
const resetPath = path.join(stateDir, "reset-generation");
const inferenceAgentPath = path.join(stateDir, "inference-agent");
const failNextOperationPath = path.join(stateDir, "inference-fail-next");

function localChildEnvironment(): NodeJS.ProcessEnv {
  const environment: NodeJS.ProcessEnv = { NODE_ENV: "development" };
  for (const key of [
    "PATH",
    "HOME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SHELL",
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    "FORCE_COLOR",
    "LANG",
    "LC_ALL",
    "TZ",
    "SSL_CERT_FILE",
    "NIX_SSL_CERT_FILE",
    "NIX_PATH",
    "NIX_PROFILES",
    "IN_NIX_SHELL",
  ]) {
    const value = process.env[key];
    if (value !== undefined) environment[key] = value;
  }
  return environment;
}

function parseScenario(value: string | undefined): Scenario {
  if (!value || !VALID_SCENARIOS.has(value)) {
    throw new Error(`scenario must be one of: ${[...VALID_SCENARIOS].join(", ")}`);
  }
  return value as Scenario;
}

function writeScenario(scenario: Scenario) {
  fs.writeFileSync(scenarioPath, `${scenario}\n`, { mode: 0o600 });
}

function usage() {
  return "usage: web-design-fixture.ts [serve | set-scenario healthy|unavailable|recovering | reset"
    + ` | set-agent ${FAKE_AGENTS.join("|")} [${FAKE_ROUTES.join("|")} [unconfirmed]]`
    + ` | fail-next-operation [${OPERATION_ERROR_CODES.join("|")}]]`;
}

function runFixtureCommand(command: string | undefined, argument?: string, secondArgument?: string, thirdArgument?: string) {
  if (command === "set-scenario") {
    const scenario = parseScenario(argument);
    fs.mkdirSync(stateDir, { recursive: true });
    writeScenario(scenario);
    console.log(`Web design fixture scenario: ${scenario}`);
    process.exit(0);
  }
  if (command === "reset") {
    fs.mkdirSync(stateDir, { recursive: true });
    fs.rmSync(statePath, { force: true });
    fs.rmSync(`${statePath}.tmp`, { force: true });
    fs.rmSync(attachmentsDir, { force: true, recursive: true });
    writeScenario("healthy");
    fs.writeFileSync(resetPath, `${process.hrtime.bigint()}\n`, { mode: 0o600 });
    console.log("Reset only the local web design fixture state.");
    process.exit(0);
  }
  if (command === "set-agent") {
    const choice = parseInferenceChoice(`${argument ?? ""} ${secondArgument ?? ""} ${thirdArgument ?? ""}`);
    if (!choice) throw new Error(usage());
    const unconfirmed = choice.unconfirmed ? " unconfirmed" : "";
    fs.mkdirSync(stateDir, { recursive: true });
    fs.writeFileSync(inferenceAgentPath, `${choice.agent} ${choice.saved}${unconfirmed}\n`, { mode: 0o600 });
    console.log(
      `Web design fixture agent: ${choice.agent}, saved route ${choice.saved}${choice.unconfirmed ? ", facts not confirmed" : ""}.`
        + " A running fixture restarts its Connections state."
    );
    process.exit(0);
  }
  if (command === "fail-next-operation") {
    const code = argument ?? "verify_failed";
    if (!isOperationErrorCode(code)) throw new Error(usage());
    fs.mkdirSync(stateDir, { recursive: true });
    fs.writeFileSync(failNextOperationPath, `${code}\n`, { mode: 0o600 });
    console.log(`The next Connections operation in the running fixture fails with ${code}.`);
    process.exit(0);
  }
  if (command && command !== "serve") {
    throw new Error(usage());
  }
  let runtimeCommandsUrl: string | null;
  try {
    runtimeCommandsUrl = runtimeCommandsForwardUrl(process.env.FC_DESIGN_RUNTIME_COMMANDS_URL);
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
  void serve(runtimeCommandsUrl);
}

async function serve(runtimeCommandsUrl: string | null) {
  fs.mkdirSync(stateDir, { recursive: true });
  if (!fs.existsSync(scenarioPath)) {
    writeScenario("healthy");
  }
  // A failure armed for an earlier run must not surprise this one.
  fs.rmSync(failNextOperationPath, { force: true });

  let state = loadState();
  let inference = createInferenceFake(readInferenceChoice());
  let recoveringFailuresRemaining = 2;
  const streams = new Map<ServerResponse, URLSearchParams>();
  const attachments = loadAttachments(state);
  const closeStreams = () => {
    for (const stream of streams.keys()) stream.end();
    streams.clear();
  };

  fs.watchFile(scenarioPath, { interval: 100 }, () => {
    recoveringFailuresRemaining = 2;
    closeStreams();
  });
  fs.watchFile(resetPath, { interval: 100 }, () => {
    state = initialState();
    inference = createInferenceFake(readInferenceChoice());
    recoveringFailuresRemaining = 2;
    closeStreams();
  });
  fs.watchFile(inferenceAgentPath, { interval: 100 }, () => {
    inference = createInferenceFake(readInferenceChoice());
  });
  fs.watchFile(failNextOperationPath, { interval: 100 }, () => {
    const code = readFailNextOperation();
    if (!code) return;
    inference.failNextOperation(code);
    fs.rmSync(failNextOperationPath, { force: true });
  });
  const hostedServer = http.createServer(async (request, response) => {
    try {
      await handleHostedRequest(request, response);
    } catch (error) {
      writeJson(response, 500, { error: String(error) });
    }
  });
  hostedServer.listen(0, "127.0.0.1");
  await once(hostedServer, "listening");
  const hostedPort = listeningPort(hostedServer);

  const coreServer = http.createServer((request, response) => {
    if (
      request.headers.authorization !== `Bearer ${CORE_TOKEN}` &&
      request.headers.authorization !== "Bearer web-design-access-token"
    ) {
      writeJson(response, 401, { error: "missing fixture credential" });
      return;
    }
    if (request.method === "GET" && request.url === "/api/core/v1/me") {
      writeJson(response, 200, coreMe());
      return;
    }
    if (
      request.method === "GET" &&
      request.url === `/api/core/v1/me/runtime-routes/${MACHINE_ID}`
    ) {
      writeJson(response, 200, {
        project_id: "project_web_design",
        runtime_id: RUNTIME_ID,
      });
      return;
    }
    if (request.method === "GET" && request.url === "/api/core/v1/me/billing") {
      writeJson(response, 200, {
        customer_org: {
          id: "org_web_design",
          owner_user_id: WORKOS_USER_ID,
          name: "Design Fixture",
          billing_class: "sponsored",
          created_at: "2026-07-01T12:00:00Z",
          updated_at: "2026-07-01T12:00:00Z",
        },
        billing_account: null,
        agent_creation_entitlement: null,
        can_create_agent: false,
        requires_billing: false,
      });
      return;
    }

    const runtimeControl = request.url?.match(
      /^\/api\/core\/v1\/me\/projects\/([^/]+)\/runtime\/(restart|recover-known-good-chat)$/u
    );
    if (request.method === "POST" && runtimeControl?.[1] && runtimeControl[2]) {
      const projectId = decodeURIComponent(runtimeControl[1]);
      const kind = runtimeControl[2];
      if (projectId !== "project_web_design") {
        writeJson(response, 404, { error: "project not found" });
        return;
      }
      if (kind === "recover-known-good-chat") {
        writeScenario("recovering");
        recoveringFailuresRemaining = 2;
      }
      const now = new Date().toISOString();
      writeJson(response, 200, {
        id: `runtime_control_${kind}`,
        project_id: projectId,
        agent_runtime_id: RUNTIME_ID,
        source_host_id: "design-fixture",
        source_machine_id: MACHINE_ID,
        requested_by_user_id: WORKOS_USER_ID,
        kind: kind === "restart" ? "restart" : "recover_known_good_chat_runtime",
        status: "requested",
        created_at: now,
        updated_at: now,
      });
      return;
    }

    writeJson(response, 404, { error: "not found" });
  });
  coreServer.listen(0, "127.0.0.1");
  await once(coreServer, "listening");
  const corePort = listeningPort(coreServer);

  const dashboardPort = Number(process.env.FC_WEB_DESIGN_PORT ?? DEFAULT_PORT);
  if (!Number.isInteger(dashboardPort) || dashboardPort < 1 || dashboardPort > 65_535) {
    throw new Error("FC_WEB_DESIGN_PORT must be a TCP port between 1 and 65535");
  }
  const dashboard = spawn(
    process.execPath,
    [
      "node_modules/next/dist/bin/next",
      "dev",
      "--hostname",
      "127.0.0.1",
      "--port",
      String(dashboardPort),
    ],
    {
      cwd: dashboardDir,
      env: {
        ...localChildEnvironment(),
        FC_CORE_API_TOKEN: CORE_TOKEN,
        FC_CORE_BASE_URL: `http://127.0.0.1:${corePort}`,
        FINITECHAT_HOSTED_API_TOKEN: DEVICE_TOKEN,
        FC_HOSTED_WEB_DEVICE_URL: `http://127.0.0.1:${hostedPort}`,
        FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
        FC_DASHBOARD_DEV_EMAIL: FIXTURE_EMAIL,
        FC_DASHBOARD_DEV_WORKOS_USER_ID: WORKOS_USER_ID,
        FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "web-design-access-token",
        FC_DASHBOARD_RUNTIME_MODE: "canary",
        FC_DASHBOARD_BASE_URL: `http://127.0.0.1:${dashboardPort}`,
        FC_WORKOS_AUTH_ENABLED: "0",
        WORKOS_COOKIE_PASSWORD: "web-design-cookie-password-32-characters",
        NEXT_DIST_DIR: ".next-web-design",
        NEXT_PUBLIC_FC_DESIGN_PREVIEWS: process.env.FC_WEB_DESIGN_PREVIEWS === "0" ? "0" : "1",
        NEXT_TELEMETRY_DISABLED: "1",
        NODE_OPTIONS:
          process.env.FC_WEB_DESIGN_NODE_OPTIONS ?? "--max-old-space-size=2048",
      },
      stdio: "inherit",
    },
  );

  console.log(
    `\nReal dashboard UI: http://127.0.0.1:${dashboardPort}/dashboard/machines/${RUNTIME_ID}/chat`
  );
  console.log("State survives stopping and restarting this command.");
  console.log(
    "Switch states in another terminal with: just dev web-design-state healthy|unavailable|recovering"
  );
  console.log(
    runtimeCommandsUrl
      ? `Connections runtime commands are forwarded to ${runtimeCommandsUrl}\n`
      : `Connections agent: node --import tsx scripts/web-design-fixture.ts set-agent ${FAKE_AGENTS.join("|")} [route [unconfirmed]]`
        + " (fail the next operation with fail-next-operation)\n"
  );

  let shuttingDown = false;
  for (const signal of ["SIGINT", "SIGTERM"] as const) {
    process.on(signal, () => shutdown(signal));
  }
  dashboard.on("exit", (code) => {
    if (!shuttingDown) {
      unwatchControlFiles();
      hostedServer.close();
      coreServer.close();
      process.exit(code ?? 1);
    }
  });

async function handleHostedRequest(request: IncomingMessage, response: ServerResponse) {
  const requestPath = new URL(request.url ?? "/", "http://fixture").pathname;
  if (request.method === "GET" && requestPath === "/runtime-status") {
    writeJson(response, 200, { agent_npub: "npub1webdesignfixture" });
    return;
  }
  if (!requestPath.startsWith("/v1/app/")) {
    writeJson(response, 404, { error: "not found" });
    return;
  }
  if (request.headers.authorization !== `Bearer ${DEVICE_TOKEN}` || request.headers["x-finite-workos-user-id"] !== WORKOS_USER_ID) {
    writeJson(response, 401, { error: "missing fixture credential" });
    return;
  }

  const scenario = readScenario();
  if (scenario !== "recovering") recoveringFailuresRemaining = 2;
  const intentionallyUnavailable =
    scenario === "unavailable" ||
    (scenario === "recovering" && recoveringFailuresRemaining-- > 0);
  if (intentionallyUnavailable && (requestPath === "/v1/app/state" || requestPath === "/v1/app/updates")) {
    writeJson(response, 503, {
      error: "The local design fixture is simulating a recoverable chat interruption.",
    });
    return;
  }

  if (
    request.method === "POST" &&
    (requestPath === "/v1/app/agent-bindings/open" ||
      requestPath === "/v1/app/agent-bindings/ensure")
  ) {
    const body = (await readJson(request)) as Record<string, unknown>;
    if (body.project_id !== "project_web_design") {
      writeJson(response, 404, { error: "agent binding not found" });
      return;
    }
    writeJson(response, 200, appState());
    return;
  }

  if (request.method === "GET" && requestPath === "/v1/app/state") {
    writeJson(response, 200, appState(new URL(request.url ?? "/", "http://fixture").searchParams));
    return;
  }
  if (request.method === "GET" && requestPath.startsWith("/v1/app/attachments/")) {
    const attachmentId = requestPath.split("/").at(-1) ?? "";
    const attachment = attachments.get(attachmentId);
    if (!attachment) {
      writeJson(response, 404, { error: "attachment not found" });
      return;
    }
    response.writeHead(200, {
      "cache-control": "private, no-store",
      "content-disposition": `inline; filename="${attachment.filename.replaceAll('"', "_")}"`,
      "content-length": String(attachment.bytes.length),
      "content-type": attachment.mimeType,
      "x-content-type-options": "nosniff",
    });
    response.end(attachment.bytes);
    return;
  }
  if (request.method === "GET" && requestPath === "/v1/app/updates") {
    response.writeHead(200, {
      "cache-control": "no-cache",
      connection: "keep-alive",
      "content-type": "text/event-stream",
    });
    const view = new URL(request.url ?? "/", "http://fixture").searchParams;
    streams.set(response, view);
    response.on("close", () => streams.delete(response));
    writeEvent(response, view);
    return;
  }
  if (request.method === "POST" && requestPath === "/v1/app/new-chat") {
    applyAction({ StartTopicChatIntent: await readJson(request) });
    writeJson(response, 200, appState());
    emitState();
    return;
  }
  if (request.method === "POST" && requestPath === "/v1/app/actions") {
    const action = await readJson(request);
    applyAction(action as Record<string, unknown>);
    writeJson(response, 200, appState());
    emitState();
    return;
  }
  if (
    request.method === "POST"
    && requestPath === "/v1/app/agent-bindings/open"
  ) {
    writeJson(response, 200, appState());
    return;
  }
  if (request.method === "POST" && requestPath === "/v1/app/attachments") {
    const contentType = request.headers["content-type"] ?? "";
    const body = await readBytes(request);
    const formData = await new Response(body, {
      headers: { "content-type": contentType },
    }).formData();
    const file = formData.getAll("files").find((value) => typeof value !== "string");
    if (!file) {
      writeJson(response, 400, { error: "fixture attachment is missing" });
      return;
    }
    const attachmentId = `attachment_design_${state.rev}`;
    const filename = file.name || "recording";
    const mimeType = file.type || "application/octet-stream";
    const bytes = Buffer.from(await file.arrayBuffer());
    persistAttachment(attachmentId, bytes);
    attachments.set(attachmentId, {
      bytes,
      filename,
      mimeType,
    });
    const caption = String(formData.get("caption") ?? "").trim();
    state.messages.push(
      attachmentMessage(
        caption,
        state.messages.length + 1,
        attachmentId,
        filename,
        mimeType
      )
    );
    state.messages.push(
      message(
        "This is a deterministic fixture reply to your attachment.",
        false,
        state.messages.length + 1
      )
    );
    state.rev += 1;
    saveState();
    writeJson(response, 200, appState());
    emitState();
    return;
  }
  if (request.method === "POST" && requestPath === "/v1/app/runtime-commands") {
    if (runtimeCommandsUrl) {
      await forwardRuntimeCommand(runtimeCommandsUrl, request, response);
      return;
    }
    writeJson(response, 200, inference.runtimeCommand(await readJson(request)));
    return;
  }
  writeJson(response, 404, { error: "not found" });
}

function applyAction(action: Record<string, unknown>) {
  const start = action.StartTopicChatIntent as { room_id?: string; topic_id?: string; intent_key?: string } | undefined;
  if (start?.room_id === "room_design" && start.topic_id === "home" && start.intent_key) {
    state.newChatIntents ??= {};
    state.newChatIntents[start.intent_key] ??= `chat_new_${state.rev + 1}`;
    state.selectedNewChatId = state.newChatIntents[start.intent_key];
  }
  const open = action.OpenChat as { chat_id?: string } | undefined;
  if (open?.chat_id) state.selectedNewChatId = open.chat_id === "chat_design" ? null : open.chat_id;
  const send = action.SendChatMessage as { text?: unknown; topic_id?: string; chat_id?: string } | undefined;
  if (send && typeof send.text === "string" && send.text.trim()) {
    const address = { conversation_id: send.topic_id ?? "topic_design", chat_id: send.chat_id ?? "chat_design" };
    state.messages.push({ ...message(send.text.trim(), true, state.messages.length + 1), ...address });
    state.messages.push({ ...message("This is a deterministic fixture reply from your recovered local chat.", false, state.messages.length + 1), ...address });
  }
  const rename = action.RenameChat as { title?: unknown } | undefined;
  if (rename && typeof rename.title === "string" && rename.title.trim()) {
    // The current real UI gets the renamed title through the same state payload.
    state.chatTitle = rename.title.trim().slice(0, 120);
  }
  const archive = action.SetChatArchived as { archived?: unknown } | undefined;
  if (archive && typeof archive.archived === "boolean") {
    state.chatArchived = archive.archived;
  }
  const createTopic = action.CreateTopic as { title?: unknown } | undefined;
  if (createTopic && typeof createTopic.title === "string" && createTopic.title.trim()) {
    const createdSeq = state.rev + 1;
    state.extraTopics.push({
      topicId: `topic_fixture_${createdSeq}`,
      title: createTopic.title.trim().slice(0, 120),
      createdSeq,
    });
  }
  state.rev += 1;
  saveState();
}

function appState(view = new URLSearchParams()) {
  const selectedChat = view.get("chat_id") ?? state.selectedNewChatId ?? "chat_design";
  const selectedTopic = view.get("topic_id") ?? (selectedChat === "chat_design" ? "topic_design" : "home");
  const messages = state.messages.filter((message) => message.chat_id === selectedChat);
  const anchorIndex = messages.findIndex(message => message.message_id === view.get("oldest_message_id"));
  const limit = Math.max(Number(view.get("limit") ?? 50), anchorIndex < 0 ? 0 : messages.length - anchorIndex);
  const last = state.messages.at(-1)?.display_content ?? "Chat restored";
  return {
    rev: state.rev,
    identity: { account_id: "web-design-user", device_id: "hosted-web" },
    rooms: [{ room_id: "room_design", display_name: "Moss", state: "Connected", status: "Connected", user_status_text: "Connected", last_message_preview: last, unread_count: 0, is_agent_chat: true, can_load_older: messages.length > limit }],
    selected_room_id: "room_design",
    topics: [
      {
        room_id: "room_design", topic_id: "home", title: "Home",
        last_message_preview: "", unread_count: 0, message_count: 0,
        created_seq: 0, updated_seq: state.rev, archived: false,
        active_chat_id: state.selectedNewChatId ?? null,
        chats: Object.values(state.newChatIntents ?? {}).map((chatId) => ({
          chat_id: chatId, title: "New chat", active: chatId === state.selectedNewChatId, archived: false,
        })),
      },
      {
        room_id: "room_design",
        topic_id: "topic_design",
        title: "General",
        last_message_preview: last,
        unread_count: 0,
        message_count: state.messages.length,
        created_seq: 1,
        updated_seq: state.rev,
        archived: false,
        active_chat_id: "chat_design",
        chats: [{
          chat_id: "chat_design",
          title: state.chatTitle,
          active: true,
          archived: state.chatArchived,
        }],
      },
      ...state.extraTopics.map((topic) => ({
        room_id: "room_design",
        topic_id: topic.topicId,
        title: topic.title,
        last_message_preview: "",
        unread_count: 0,
        message_count: 0,
        created_seq: topic.createdSeq,
        updated_seq: topic.createdSeq,
        archived: false,
        active_chat_id: null,
        chats: [],
      })),
    ],
    selected_topic_id: selectedTopic,
    selected_chat_id: selectedChat,
    active_profile_id: "agent_design",
    status: "Runtime running",
    toast: null,
    messages: messages.slice(-limit),
    profiles: [{ account_id: "agent_design", npub: "npub1webdesignfixture", display_name: "Moss", about: "A deterministic local design collaborator", picture: null, stale: false, is_agent: true }],
    devices: [{ account_id: "web-design-user", device_id: "hosted-web", active: true, current_device: true, revoked: false, room_count: 1 }],
    typing_members: [],
    hosted_agent_binding: {
      version: 1,
      project_id: "project_web_design",
      human_account_id: "web-design-user",
      agent_account_id: "agent_design",
      agent_npub: "npub1webdesignfixture",
      canonical_room_id: "room_design",
      associated_room_ids: [],
    },
    flow: { notice_text: null, notice_busy: false, scan_in_flight: false, scan_result: "" },
  };
}

function coreMe() {
  return {
    email: FIXTURE_EMAIL,
    workos_user_id: WORKOS_USER_ID,
    claimable_candidates: [],
    agent_creation_requests: [],
    projects: [{
      project: { id: "project_web_design", display_name: "Moss", hosting_tier: "standard", created_at: "2026-07-01T12:00:00Z", updated_at: "2026-07-01T12:00:00Z" },
      runtime: { id: RUNTIME_ID, project_id: "project_web_design", contact_endpoint: `http://127.0.0.1:${hostedPort}/runtime-status`, runtime_status: "online", hermes_available: true, created_at: "2026-07-01T12:00:00Z", updated_at: "2026-07-01T12:00:00Z" },
    }, ...(process.env.FC_WEB_DESIGN_SECOND_AGENT === "1" ? [{
      project: { id: "project_web_design_second", display_name: "Fern", hosting_tier: "standard", created_at: "2026-07-01T12:00:00Z", updated_at: "2026-07-01T12:00:00Z" },
      runtime: { id: "runtime_web_design_second", project_id: "project_web_design_second", runtime_status: "offline", hermes_available: true, created_at: "2026-07-01T12:00:00Z", updated_at: "2026-07-01T12:00:00Z" },
    }] : [])],
  };
}

function loadState(): FixtureState {
  try {
    const parsed = JSON.parse(fs.readFileSync(statePath, "utf8")) as FixtureState;
    if (Number.isInteger(parsed.rev) && Array.isArray(parsed.messages)) {
      return {
        rev: parsed.rev,
        chatArchived: parsed.chatArchived === true,
        newChatIntents: parsed.newChatIntents ?? {},
        selectedNewChatId: parsed.selectedNewChatId ?? null,
        chatTitle:
          typeof parsed.chatTitle === "string" && parsed.chatTitle.trim()
            ? parsed.chatTitle
            : "Design review",
        extraTopics: Array.isArray(parsed.extraTopics)
          ? parsed.extraTopics.filter((topic) =>
              typeof topic?.topicId === "string"
              && typeof topic?.title === "string"
              && Number.isInteger(topic?.createdSeq)
            )
          : [],
        messages: parsed.messages,
      };
    }
  } catch {}
  return initialState();
}

function loadAttachments(state: FixtureState) {
  const attachments = new Map<string, FixtureAttachment>();
  for (const entry of state.messages) {
    const media = Array.isArray(entry.media) ? entry.media : [];
    for (const item of media) {
      if (!item || typeof item !== "object" || Array.isArray(item)) continue;
      const attachment = item as Record<string, unknown>;
      const attachmentId = attachment.attachment_id;
      const filename = attachment.filename;
      const mimeType = attachment.mime_type;
      if (
        typeof attachmentId !== "string"
        || !/^attachment_design_[0-9]+$/u.test(attachmentId)
        || typeof filename !== "string"
        || typeof mimeType !== "string"
      ) {
        continue;
      }
      try {
        attachments.set(attachmentId, {
          bytes: fs.readFileSync(path.join(attachmentsDir, attachmentId)),
          filename,
          mimeType,
        });
      } catch {}
    }
  }
  return attachments;
}

function persistAttachment(attachmentId: string, bytes: Buffer) {
  fs.mkdirSync(attachmentsDir, { recursive: true });
  const destination = path.join(attachmentsDir, attachmentId);
  const temporaryPath = `${destination}.tmp`;
  fs.writeFileSync(temporaryPath, bytes, { mode: 0o600 });
  fs.renameSync(temporaryPath, destination);
}

function initialState(): FixtureState {
  return {
    rev: 1,
    chatArchived: false,
    chatTitle: "Design review",
    extraTopics: [],
    messages: [
      message("I kept this conversation after the local dashboard restarted.", false, 1),
      message("Great. Let’s refine the web chat without needing a provider account.", true, 2),
      message("Ready. This is the real dashboard UI backed by a deterministic local service fixture.", false, 3),
    ],
  };
}

function message(text: string, isMine: boolean, seq: number) {
  return { room_id: "room_design", seq, message_id: `message_${seq}`, conversation_id: "topic_design", chat_id: "chat_design", sender_account_id: isMine ? "web-design-user" : "agent_design", sender_display_name: isMine ? "You" : "Moss", text, display_content: text, kind: "message", status: "complete", final_delivery: !isMine, edit_of_message_id: null, is_mine: isMine, media: [], timestamp_unix_seconds: 1_783_000_000 + seq, display_timestamp: "10:30 AM" };
}

function attachmentMessage(
  caption: string,
  seq: number,
  attachmentId: string,
  filename: string,
  mimeType: string
) {
  return {
    ...message(caption, true, seq),
    kind: "media",
    media: [{
      attachment_id: attachmentId,
      mime_type: mimeType,
      filename,
      kind: mimeType.startsWith("audio/") ? "VoiceNote" : "File",
      width: null,
      height: null,
    }],
  };
}

function saveState() {
  const temporaryPath = `${statePath}.tmp`;
  fs.writeFileSync(temporaryPath, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
  fs.renameSync(temporaryPath, statePath);
}

function emitState() {
  for (const [stream, view] of streams) writeEvent(stream, view);
}

function writeEvent(response: ServerResponse, view: URLSearchParams) {
  response.write(`id: ${state.rev}\nevent: state\ndata: ${JSON.stringify(appState(view))}\n\n`);
}

function unwatchControlFiles() {
  for (const controlPath of [scenarioPath, resetPath, inferenceAgentPath, failNextOperationPath]) {
    fs.unwatchFile(controlPath);
  }
}

function readScenario(): Scenario {
  try {
    return parseScenario(fs.readFileSync(scenarioPath, "utf8").trim());
  } catch {
    return "healthy";
  }
}

async function readJson(request: IncomingMessage) {
  const chunks: Buffer[] = [];
  for await (const chunk of request) chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

async function readBytes(request: IncomingMessage) {
  const chunks: Buffer[] = [];
  for await (const chunk of request) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  }
  return Buffer.concat(chunks);
}

function writeJson(response: ServerResponse, status: number, body: unknown) {
  if (response.headersSent) return;
  response.writeHead(status, { "content-type": "application/json" });
  response.end(JSON.stringify(body));
}

function listeningPort(server: http.Server) {
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("fixture server did not bind TCP");
  return address.port;
}

function shutdown(signal: NodeJS.Signals) {
  if (shuttingDown) return;
  shuttingDown = true;
  unwatchControlFiles();
  closeStreams();
  hostedServer.close();
  coreServer.close();
  dashboard.kill(signal);
  setTimeout(() => process.exit(0), 2_000).unref();
}
}

// Connections states exercised by the browser suite. Keys are never retained;
// only the SHA-256 needed to render the disconnect receipt is kept.
const FAKE_AGENTS = ["pr1", "legacy"] as const;
const FAKE_ROUTES = ["finite_private", "openrouter", "openai_codex"] as const;
const OPERATION_ERROR_CODES = ["verify_failed"] as const;

export type FakeAgentKind = (typeof FAKE_AGENTS)[number];
export type FakeInferenceRoute = (typeof FAKE_ROUTES)[number];
export type OperationErrorCode = (typeof OPERATION_ERROR_CODES)[number];
export type FakeRuntimeReply = {
  request_id: string;
  status: "succeeded" | "failed";
  body: unknown;
  error: { code: string; message: string } | null;
};
export type InferenceFakeOptions = {
  agent?: FakeAgentKind;
  saved?: FakeInferenceRoute;
  unconfirmed?: boolean;
  phaseMs?: number;
  now?: () => number;
};
export type InferenceFake = {
  runtimeCommand(request: unknown): FakeRuntimeReply;
  failNextOperation(code?: OperationErrorCode): void;
};

const PR1_CAPABILITIES = ["inference.status.v2", "inference.select.v1", "inference.disconnect.v1"];
const EMPTY_REQUEST_SCHEMA = "finite.agent.empty.request.v1";
const INFERENCE_COMMANDS = new Map<string, { schema: string; capability: string | null }>([
  ["agent.connections.status", { schema: EMPTY_REQUEST_SCHEMA, capability: null }],
  ["agent.inference.apply", { schema: "finite.agent.inference.apply.v1", capability: null }],
  ["agent.inference.select", { schema: "finite.agent.inference.select.v1", capability: "inference.select.v1" }],
  ["agent.inference.disconnect", { schema: "finite.agent.inference.disconnect.v1", capability: "inference.disconnect.v1" }],
  // No fixture agent advertises these capabilities, so each command is refused as the daemon refuses it.
  ["agent.openrouter.usage", { schema: EMPTY_REQUEST_SCHEMA, capability: "openrouter.usage.v1" }],
  ["agent.openrouter.connect", { schema: "finite.agent.openrouter.connect.v1", capability: "openrouter.connect.v1" }],
  ["agent.codex.login.start", { schema: "finite.agent.codex.login.start.v1", capability: "codex.login.v1" }],
  ["agent.codex.login.cancel", { schema: "finite.agent.codex.login.cancel.v1", capability: "codex.login.v1" }],
  ["agent.codex.models", { schema: EMPTY_REQUEST_SCHEMA, capability: "codex.models.v1" }],
]);
const OPERATION_PHASES = {
  select: ["accepted", "config_written", "restarting", "verifying"],
  disconnect: ["accepted", "login_cancelled", "route_switched", "credential_removed", "cleanup", "verifying"],
} as const;
const FAKE_FINITE_PRIVATE_MODEL = "glm-5-3-flash";
const FAKE_OPENROUTER_MODEL = "anthropic/claude-sonnet-4.6";
const FAKE_CODEX_MODEL = "gpt-5.5";
const FAKE_SEEDED_OPENROUTER_KEY = "sk-or-v1-web-design-fixture-fake-key";
const LAST_RESULT_MS = 10 * 60_000;

type SavedModel = { route: FakeInferenceRoute; model: string };
type OperationKind = keyof typeof OPERATION_PHASES;
type FakeOperation = {
  id: string;
  kind: OperationKind;
  route: FakeInferenceRoute;
  model: string | null;
  phaseIndex: number;
  state: "running" | "succeeded" | "failed";
  errorCode: OperationErrorCode | null;
  attempts: number;
  startedAtMs: number;
  updatedAtMs: number;
  shouldFail: boolean;
};

class FakeCommandError extends Error {
  constructor(readonly code: string, message: string) {
    super(message);
  }
}

export function createInferenceFake(options: InferenceFakeOptions = {}): InferenceFake {
  const agent = options.agent ?? "pr1";
  const phaseMs = Math.max(0, options.phaseMs ?? 2_000);
  const now = options.now ?? Date.now;
  const capabilities = agent === "pr1" ? PR1_CAPABILITIES : [];
  const seeded = options.saved ?? "finite_private";
  const unconfirmed = options.unconfirmed ?? false;
  let saved: SavedModel = {
    route: seeded,
    model: seeded === "openrouter" ? FAKE_OPENROUTER_MODEL : seeded === "openai_codex" ? FAKE_CODEX_MODEL : FAKE_FINITE_PRIVATE_MODEL,
  };
  let openrouterKeyHash = seeded === "openrouter" ? sha256(FAKE_SEEDED_OPENROUTER_KEY) : null;
  let operation: FakeOperation | null = null;
  let failNext = false;
  let replies = 0;

  function runtimeCommand(request: unknown): FakeRuntimeReply {
    const record = isRecord(request) ? request : {};
    const command = typeof record.command === "string" ? record.command : "";
    const known = INFERENCE_COMMANDS.get(command);
    replies += 1;
    advance();
    try {
      if (!known) return reply("succeeded", {}, null);
      if (known.capability && !capabilities.includes(known.capability)) {
        throw new FakeCommandError("unsupported_command", `Command ${JSON.stringify(command)} is not supported.`);
      }
      if (record.schema !== known.schema) throw invalidPayload(`${command} expects schema ${known.schema}.`);
      return reply("succeeded", handle(command, record.body), null);
    } catch (error) {
      if (!(error instanceof FakeCommandError)) throw error;
      return reply("failed", null, { code: error.code, message: error.message });
    }
  }

  function reply(status: FakeRuntimeReply["status"], body: unknown, error: FakeRuntimeReply["error"]): FakeRuntimeReply {
    return { request_id: `web-design-command-${replies}`, status, body, error };
  }

  function handle(command: string, body: unknown): unknown {
    switch (command) {
      case "agent.connections.status":
        return status();
      case "agent.inference.apply":
        return applyV1(body);
      case "agent.inference.select":
        return select(body);
      case "agent.inference.disconnect":
        return disconnect(body);
      default:
        throw new Error(`unhandled fake command ${command}`);
    }
  }

  function advance() {
    const at = now();
    const current = operation;
    if (!current) return;
    const phases = OPERATION_PHASES[current.kind];
    while (current.state === "running") {
      const due = current.startedAtMs + (current.phaseIndex + 1) * phaseMs;
      if (due > at) return;
      current.updatedAtMs = due;
      if (current.phaseIndex + 1 < phases.length) {
        current.phaseIndex += 1;
        const phase = phases[current.phaseIndex];
        if (phase === "config_written") {
          saved = { route: current.route, model: current.model ?? FAKE_FINITE_PRIVATE_MODEL };
        } else if (phase === "route_switched" && saved.route === current.route) {
          saved = { route: "finite_private", model: FAKE_FINITE_PRIVATE_MODEL };
        } else if (phase === "credential_removed") {
          openrouterKeyHash = null;
        }
      } else {
        current.state = current.shouldFail ? "failed" : "succeeded";
        current.errorCode = current.shouldFail ? "verify_failed" : null;
        current.attempts = current.shouldFail ? 3 : 0;
        current.shouldFail = false;
      }
    }
  }

  function admitMutation() {
    if (!operation || operation.state === "succeeded") return;
    if (operation.state === "running" || operation.kind === "disconnect") throw operationInProgress();
  }

  /** As in the daemon, a failed select stays in status until a later change is applied or is a no-op. */
  function clearFailedOperation() {
    if (operation?.state === "failed") operation = null;
  }

  function startOperation(kind: OperationKind, route: FakeInferenceRoute, model: string | null) {
    const at = now();
    operation = {
      id: `op_${randomBytes(16).toString("hex")}`,
      kind,
      route,
      model,
      phaseIndex: 0,
      state: "running",
      errorCode: null,
      attempts: 0,
      startedAtMs: at,
      updatedAtMs: at,
      shouldFail: failNext,
    };
    failNext = false;
    return { accepted: true, operation_id: operation.id };
  }

  function status() {
    const legacy = {
      profile: saved.route === "openrouter" ? "openrouter" : "finite_private",
      provider: savedProvider(saved.route),
      model: saved.model,
    };
    const others = {
      telegram: { connected: false, home_channel: null, pending: [], approved: [] },
      google: { connected: false, email: null },
    };
    if (agent === "legacy") return { inference: legacy, ...others };
    return {
      inference: {
        ...legacy,
        saved: { route: saved.route, provider: savedProvider(saved.route), model: saved.model },
        routes: {
          finite_private: { state: unconfirmed ? "unknown" : "configured", reason: null },
          openrouter: openrouterStatus(),
        },
        fallback: unconfirmed
          ? { state: "unknown", reason: null, model: null, extra_entries: 0 }
          : { state: "configured", reason: null, model: FAKE_FINITE_PRIVATE_MODEL, extra_entries: 0 },
        operation: operationStatus(),
      },
      ...others,
      capabilities,
    };
  }

  function openrouterStatus() {
    const hermes = unconfirmed ? "unknown" : openrouterKeyHash ? "saved_key" : "none";
    const pool = unconfirmed ? "unknown" : "none";
    if (openrouterKeyHash) {
      return { state: "key_saved", key_source: "agent", key_hash: openrouterKeyHash, hermes_key: hermes, other_pool_keys: pool };
    }
    return { state: unconfirmed ? "unknown" : "no_key", key_source: null, key_hash: null, hermes_key: hermes, other_pool_keys: pool };
  }

  function operationStatus() {
    if (!operation || (operation.state === "succeeded" && now() - operation.updatedAtMs >= LAST_RESULT_MS)) {
      return null;
    }
    return {
      id: operation.id,
      kind: operation.kind,
      route: operation.route,
      model: operation.model,
      state: operation.state,
      phase: OPERATION_PHASES[operation.kind][operation.phaseIndex],
      error_code: operation.errorCode,
      attempts: operation.attempts,
      updated_at_ms: operation.updatedAtMs,
    };
  }

  function applyV1(body: unknown) {
    const request = bodyRecord(body, ["profile", "api_key", "model"]);
    const apiKey = optionalText(request.api_key, "api_key");
    const model = optionalText(request.model, "model");
    admitMutation();
    let planned: SavedModel;
    let keyHash = openrouterKeyHash;
    if (request.profile === "finite_private") {
      planned = { route: "finite_private", model: FAKE_FINITE_PRIVATE_MODEL };
    } else if (request.profile === "openrouter") {
      if (apiKey?.trim()) keyHash = sha256(apiKey);
      if (!keyHash) throw invalidPayload("OpenRouter key is required");
      planned = { route: "openrouter", model: modelName(model ?? FAKE_OPENROUTER_MODEL) };
    } else {
      throw invalidPayload("Inference must be Finite Private or OpenRouter");
    }
    const unchanged = sameModel(planned, saved) && keyHash === openrouterKeyHash;
    if (!unchanged) {
      openrouterKeyHash = keyHash;
      saved = planned;
    }
    clearFailedOperation();
    return {
      proposal_id: `web-design-proposal-${replies}`,
      path: "model",
      applied: !unchanged,
      already_applied: unchanged,
      restart_required: !unchanged,
    };
  }

  function select(body: unknown) {
    const request = bodyRecord(body, ["route", "model"]);
    const route = request.route === "finite_private" || request.route === "openrouter" ? request.route : null;
    if (!route) throw invalidPayload("Unknown route.");
    if (route === "finite_private" && request.model !== undefined && request.model !== null) {
      throw invalidPayload("Finite Private takes no model.");
    }
    const planned: SavedModel =
      route === "finite_private"
        ? { route, model: FAKE_FINITE_PRIVATE_MODEL }
        : { route, model: modelName(request.model) };
    admitMutation();
    if (route === "openrouter" && !openrouterKeyHash) {
      throw new FakeCommandError("not_connected", "Connect OpenRouter first.");
    }
    if (sameModel(planned, saved)) {
      clearFailedOperation();
      return { changed: false };
    }
    return startOperation("select", route, route === "finite_private" ? null : planned.model);
  }

  function disconnect(body: unknown) {
    const request = bodyRecord(body, ["route"]);
    if (request.route !== "openrouter") throw invalidPayload("Unknown route.");
    if (operation?.state === "failed" && operation.kind === "disconnect") {
      const at = now();
      Object.assign(operation, {
        state: "running",
        errorCode: null,
        attempts: 0,
        startedAtMs: at - operation.phaseIndex * phaseMs,
        updatedAtMs: at,
        shouldFail: failNext,
      });
      failNext = false;
      return { accepted: true, operation_id: operation.id };
    }
    admitMutation();
    if (!openrouterKeyHash && saved.route !== "openrouter") {
      clearFailedOperation();
      return { changed: false };
    }
    return startOperation("disconnect", "openrouter", null);
  }

  return {
    runtimeCommand,
    failNextOperation() {
      failNext = true;
    },
  };
}

function sha256(value: string) {
  return createHash("sha256").update(value).digest("hex");
}

function savedProvider(route: FakeInferenceRoute) {
  return route === "openrouter" ? "openrouter" : route === "openai_codex" ? "openai-codex" : "custom";
}

function sameModel(left: SavedModel, right: SavedModel) {
  return left.route === right.route && left.model === right.model;
}

function modelName(value: unknown) {
  if (typeof value !== "string" || value.length < 1 || value.length > 256 || /[\s\p{Cc}]/u.test(value)) {
    throw invalidPayload("model is invalid.");
  }
  return value;
}

function optionalText(value: unknown, field: string) {
  if (value === undefined || value === null) return undefined;
  if (typeof value !== "string") throw invalidPayload(`${field} is invalid.`);
  return value;
}

function bodyRecord(value: unknown, allowed: readonly string[]) {
  if (!isRecord(value)) throw invalidPayload("The command body must be an object.");
  const unknown = Object.keys(value).find((key) => !allowed.includes(key));
  if (unknown) throw invalidPayload(`Unknown field ${JSON.stringify(unknown)}.`);
  return value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function invalidPayload(message: string) {
  return new FakeCommandError("invalid_payload", message);
}

function operationInProgress() {
  return new FakeCommandError("operation_in_progress", "Another connection change is still finishing. Try again in a moment.");
}

function isOperationErrorCode(value: string): value is OperationErrorCode {
  return (OPERATION_ERROR_CODES as readonly string[]).includes(value);
}

export function parseInferenceChoice(
  text: string
): { agent: FakeAgentKind; saved: FakeInferenceRoute; unconfirmed: boolean } | null {
  const [agentName, route = "finite_private", facts, ...rest] = text.trim().split(/\s+/u);
  const agent = FAKE_AGENTS.find((value) => value === agentName);
  const saved = FAKE_ROUTES.find((value) => value === route);
  if (
    rest.length > 0 ||
    (facts !== undefined && facts !== "unconfirmed") ||
    (saved === "openai_codex" && agent !== "legacy")
  ) return null;
  return agent && saved ? { agent, saved, unconfirmed: facts === "unconfirmed" } : null;
}

function readInferenceChoice() {
  try {
    return parseInferenceChoice(fs.readFileSync(inferenceAgentPath, "utf8")) ?? {};
  } catch {
    return {};
  }
}

function readFailNextOperation() {
  try {
    const code = fs.readFileSync(failNextOperationPath, "utf8").trim();
    return isOperationErrorCode(code) ? code : null;
  } catch {
    return null;
  }
}

const LOCAL_RUNTIME_COMMANDS_URL = /^http:\/\/(127\.0\.0\.1|localhost):([0-9]{1,5})\/?$/u;

/**
 * `FC_DESIGN_RUNTIME_COMMANDS_URL` names a local harness (DESIGN E-0) that answers
 * `/v1/app/runtime-commands` in place of the fakes. Only a loopback origin is accepted, so the
 * fixture can never be pointed at a real service by accident.
 */
export function runtimeCommandsForwardUrl(value: string | undefined): string | null {
  if (value === undefined || value === "") return null;
  const match = LOCAL_RUNTIME_COMMANDS_URL.exec(value);
  const port = Number(match?.[2]);
  if (!match || port < 1 || port > 65_535) {
    throw new Error(
      "FC_DESIGN_RUNTIME_COMMANDS_URL must be http://127.0.0.1:<port> or http://localhost:<port>. "
        + "The design fixture forwards runtime commands only to a local harness."
    );
  }
  return `http://${match[1]}:${port}/v1/app/runtime-commands`;
}

export async function forwardRuntimeCommand(target: string, request: IncomingMessage, response: ServerResponse) {
  const chunks: Buffer[] = [];
  for await (const chunk of request) chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  const headers: Record<string, string> = { "content-type": "application/json" };
  for (const name of ["authorization", "x-finite-workos-user-id"]) {
    const value = request.headers[name];
    if (typeof value === "string") headers[name] = value;
  }
  const upstream = await fetch(target, {
    method: "POST",
    headers,
    body: new Uint8Array(Buffer.concat(chunks)),
    signal: AbortSignal.timeout(65_000),
  });
  response.writeHead(upstream.status, {
    "content-type": upstream.headers.get("content-type") ?? "application/json",
  });
  response.end(Buffer.from(await upstream.arrayBuffer()));
}

function invokedDirectly() {
  try {
    return Boolean(process.argv[1])
      && fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
}

// Last, so every constant above is initialized. Tests import this file for its fakes without starting it.
if (invokedDirectly()) {
  runFixtureCommand(process.argv[2], process.argv[3], process.argv[4], process.argv[5]);
}
