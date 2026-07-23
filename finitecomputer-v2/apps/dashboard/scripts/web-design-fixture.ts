import { spawn } from "node:child_process";
import { once } from "node:events";
import fs from "node:fs";
import http, { type IncomingMessage, type ServerResponse } from "node:http";
import path from "node:path";

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
  chatTitle: string;
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

const command = process.argv[2];
if (command === "set-scenario") {
  const scenario = parseScenario(process.argv[3]);
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
if (command && command !== "serve") {
  throw new Error("usage: web-design-fixture.ts [serve | set-scenario healthy|unavailable|recovering | reset]");
}

void serve();

async function serve() {
  fs.mkdirSync(stateDir, { recursive: true });
  if (!fs.existsSync(scenarioPath)) {
    writeScenario("healthy");
  }

  let state = loadState();
  let recoveringFailuresRemaining = 2;
  const streams = new Set<ServerResponse>();
  const attachments = loadAttachments(state);
  const closeStreams = () => {
    for (const stream of streams) stream.end();
    streams.clear();
  };

  fs.watchFile(scenarioPath, { interval: 100 }, () => {
    recoveringFailuresRemaining = 2;
    closeStreams();
  });
  fs.watchFile(resetPath, { interval: 100 }, () => {
    state = initialState();
    recoveringFailuresRemaining = 2;
    closeStreams();
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
    "Switch states in another terminal with: just dev web-design-state healthy|unavailable|recovering\n"
  );

  let shuttingDown = false;
  for (const signal of ["SIGINT", "SIGTERM"] as const) {
    process.on(signal, () => shutdown(signal));
  }
  dashboard.on("exit", (code) => {
    if (!shuttingDown) {
      fs.unwatchFile(scenarioPath);
      fs.unwatchFile(resetPath);
      hostedServer.close();
      coreServer.close();
      process.exit(code ?? 1);
    }
  });

async function handleHostedRequest(request: IncomingMessage, response: ServerResponse) {
  const requestPath = request.url ?? "/";
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

  if (request.method === "GET" && requestPath === "/v1/app/state") {
    writeJson(response, 200, appState());
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
    streams.add(response);
    response.on("close", () => streams.delete(response));
    writeEvent(response);
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
    writeJson(response, 200, {
      request_id: "web-design-command",
      status: "succeeded",
      body: {},
      error: null,
    });
    return;
  }
  writeJson(response, 404, { error: "not found" });
}

function applyAction(action: Record<string, unknown>) {
  const send = action.SendChatMessage as { text?: unknown } | undefined;
  if (send && typeof send.text === "string" && send.text.trim()) {
    state.messages.push(message(send.text.trim(), true, state.messages.length + 1));
    state.messages.push(message("This is a deterministic fixture reply from your recovered local chat.", false, state.messages.length + 1));
  }
  const rename = action.RenameChat as { title?: unknown } | undefined;
  if (rename && typeof rename.title === "string" && rename.title.trim()) {
    // The current real UI gets the renamed title through the same state payload.
    state.chatTitle = rename.title.trim().slice(0, 120);
  }
  state.rev += 1;
  saveState();
}

function appState() {
  const last = state.messages.at(-1)?.display_content ?? "Chat restored";
  return {
    rev: state.rev,
    identity: { account_id: "web-design-user", device_id: "hosted-web" },
    rooms: [{ room_id: "room_design", display_name: "Moss", state: "Connected", status: "Connected", user_status_text: "Connected", last_message_preview: last, unread_count: 0, is_agent_chat: true }],
    selected_room_id: "room_design",
    topics: [
      {
        room_id: "room_design",
        topic_id: "topic_design",
        title: "General",
        active_chat_id: "chat_design",
        chats: [{ chat_id: "chat_design", title: state.chatTitle, active: true }],
      },
    ],
    selected_topic_id: "topic_design",
    selected_chat_id: "chat_design",
    active_profile_id: "agent_design",
    status: "Runtime running",
    toast: null,
    messages: state.messages,
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
    }],
  };
}

function loadState(): FixtureState {
  try {
    const parsed = JSON.parse(fs.readFileSync(statePath, "utf8")) as FixtureState;
    if (Number.isInteger(parsed.rev) && Array.isArray(parsed.messages)) {
      return {
        rev: parsed.rev,
        chatTitle:
          typeof parsed.chatTitle === "string" && parsed.chatTitle.trim()
            ? parsed.chatTitle
            : "Design review",
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
    chatTitle: "Design review",
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
  for (const stream of streams) writeEvent(stream);
}

function writeEvent(response: ServerResponse) {
  response.write(`id: ${state.rev}\nevent: state\ndata: ${JSON.stringify(appState())}\n\n`);
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
  fs.unwatchFile(scenarioPath);
  fs.unwatchFile(resetPath);
  closeStreams();
  hostedServer.close();
  coreServer.close();
  dashboard.kill(signal);
  setTimeout(() => process.exit(0), 2_000).unref();
}
}
