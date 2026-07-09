#!/usr/bin/env node
import { _electron as electron } from "playwright";
import { spawnSync } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const electronExecutable = require("electron");

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const appDir = path.resolve(scriptDir, "..");
const repoRoot = path.resolve(appDir, "../..");
const targetRoot = path.resolve(repoRoot, "target", "electron-hermes-ui-smoke");
const serverUrl = process.env.FINITECHAT_E2E_SERVER_URL || process.env.FINITECHAT_SERVER_URL || "https://chat.finite.computer";
const replyTimeoutMs = Number(process.env.FINITECHAT_E2E_REPLY_TIMEOUT_MS || 240_000);
const connectTimeoutMs = Number(process.env.FINITECHAT_E2E_CONNECT_TIMEOUT_MS || 90_000);
const keepApp = process.env.FINITECHAT_E2E_KEEP_APP === "1";
const runId = new Date().toISOString().replace(/[-:.TZ]/g, "").slice(0, 14);
const report = {
  run_id: runId,
  server_url: serverUrl,
  steps: [],
  facts: {},
};

let electronApp = null;
let page = null;
let daemonUrl = null;
let daemonPort = null;
let artifactDir = null;

if (!existsSync(path.join(appDir, "dist", "index.html"))) {
  console.error("Renderer build not found. Run `npm run build` before this smoke.");
  process.exit(1);
}

main().catch(async (error) => {
  report.error = error instanceof Error ? error.stack || error.message : String(error);
  await captureFailureArtifacts();
  await writeReport();
  await closeApp();
  console.error(`Electron Hermes UI smoke failed: ${error instanceof Error ? error.message : String(error)}`);
  if (artifactDir) {
    console.error(`Artifacts: ${artifactDir}`);
  }
  process.exit(1);
});

async function main() {
  artifactDir = path.join(targetRoot, runId);
  await mkdir(artifactDir, { recursive: true });
  const userDataDir = await mkdtemp(path.join(artifactDir, "user-data-"));
  daemonPort = await freePort();
  daemonUrl = `http://127.0.0.1:${daemonPort}`;
  const deviceId = `electron-ui-smoke-${runId}`;
  const hermes = resolveHermesTarget();

  report.facts.artifact_dir = artifactDir;
  report.facts.user_data_dir = userDataDir;
  report.facts.daemon_url = daemonUrl;
  report.facts.device_id = deviceId;
  report.facts.hermes_target_source = hermes.source;
  report.facts.hermes_container_id = hermes.container_id || null;
  report.facts.hermes_npub = npubFromTarget(hermes.target);
  step("resolved_hermes_target", {
    source: hermes.source,
    container_id: hermes.container_id || null,
    npub: report.facts.hermes_npub,
  });

  electronApp = await electron.launch({
    executablePath: electronExecutable,
    args: [appDir],
    cwd: appDir,
    env: {
      ...process.env,
      FINITECHAT_DAEMON_URL: daemonUrl,
      FINITECHAT_START_DAEMON: "1",
      FINITECHAT_USER_DATA_DIR: userDataDir,
      FINITECHAT_DEVICE_ID: deviceId,
      FINITECHAT_SERVER_URL: serverUrl,
      FINITECHAT_DISABLE_SINGLE_INSTANCE_LOCK: "1",
      FINITECHAT_SKIP_PROTOCOL_REGISTRATION: "1",
    },
  });
  step("electron_launched");

  page = await electronApp.firstWindow({ timeout: 60_000 });
  page.setDefaultTimeout(60_000);
  page.on("console", (message) => {
    const type = message.type();
    if (type === "error" || type === "warning") {
      report.steps.push({
        name: "renderer_console",
        type,
        message: message.text(),
        at: new Date().toISOString(),
      });
    }
  });
  await page.waitForLoadState("domcontentloaded");
  await waitFor("daemon health", 60_000, async () => {
    const health = await daemonJson("/v1/healthz");
    return health?.status === "ok" ? health : null;
  });
  step("daemon_healthy");

  await finishOnboarding();
  await connectHermes(hermes.target);
  const connected = await waitFor("connected Hermes room", connectTimeoutMs, async () => {
    const state = await appState();
    const room = state.rooms.find((candidate) => candidate.is_agent_chat && candidate.state === "Connected");
    if (!room) {
      return null;
    }
    const topic =
      state.topics.find((candidate) => candidate.room_id === room.room_id && candidate.topic_id === state.selected_topic_id) ??
      state.topics.find((candidate) => candidate.room_id === room.room_id && candidate.topic_id === "home") ??
      state.topics.find((candidate) => candidate.room_id === room.room_id);
    const chat =
      topic?.chats.find((candidate) => candidate.chat_id === state.selected_chat_id) ??
      topic?.chats.find((candidate) => candidate.active) ??
      topic?.chats[0] ??
      null;
    return { state, room, topic, chat };
  });
  report.facts.room_id = connected.room.room_id;
  report.facts.topic_id = connected.topic?.topic_id || null;
  report.facts.chat_id = connected.chat?.chat_id || null;
  report.facts.room_display_name = connected.room.display_name;
  step("hermes_room_connected", {
    room_id: connected.room.room_id,
    topic_id: connected.topic?.topic_id || null,
    chat_id: connected.chat?.chat_id || null,
  });

  await waitForComposer();
  const messageText = `Electron UI smoke ${runId}. Reply briefly so the test can confirm Hermes responded.`;
  report.facts.user_message_text = messageText;
  await page.locator("textarea").fill(messageText);
  await page.getByRole("button", { name: "Send message" }).click();
  step("message_sent_from_ui");

  const sent = await waitFor("sent message projection", 30_000, async () => {
    const state = await appState();
    return state.messages.find((message) => message.is_mine && message.text.includes(runId)) ?? null;
  });
  report.facts.user_message_id = sent.message_id;
  report.facts.user_message_seq = sent.seq;

  const reply = await waitFor("Hermes reply", replyTimeoutMs, async () => {
    const state = await appState();
    return (
      state.messages.find((message) => !message.is_mine && message.room_id === sent.room_id && message.seq > sent.seq) ?? null
    );
  });
  report.facts.reply_message_id = reply.message_id;
  report.facts.reply_seq = reply.seq;
  report.facts.reply_sender = {
    account_id: reply.sender_account_id,
    device_id: reply.sender_device_id,
    display_name: reply.sender_display_name,
    npub: reply.sender_npub || null,
  };
  report.facts.reply_preview = (reply.display_content || reply.text || "").slice(0, 500);
  step("hermes_reply_observed", {
    sender: reply.sender_display_name,
    seq: reply.seq,
  });

  await page.screenshot({ path: path.join(artifactDir, "success.png"), fullPage: true });
  await writeStateSnapshot("success-state.json");
  await writeReport();
  await closeApp();
  console.log(JSON.stringify(report, null, 2));
}

function resolveHermesTarget() {
  const explicit = process.env.FINITECHAT_E2E_HERMES_TARGET || process.env.FINITECHAT_E2E_HERMES_NPUB;
  if (explicit?.trim()) {
    return { target: explicit.trim(), source: "env" };
  }

  const containerId = process.env.FINITECHAT_E2E_CONTAINER || detectFiniteAgentContainer();
  if (!containerId) {
    throw new Error(
      "No Hermes target found. Set FINITECHAT_E2E_HERMES_TARGET to an npub/profile link, or run a finite-agent-runtime container."
    );
  }
  const npub = readContainerHermesNpub(containerId);
  if (!npub) {
    throw new Error(
      `Container ${containerId} did not expose a Hermes npub. Set FINITECHAT_E2E_HERMES_TARGET explicitly.`
    );
  }
  return { target: npub, source: "apple-container", container_id: containerId };
}

function detectFiniteAgentContainer() {
  const result = command("container", ["list", "--format", "json"], { check: false });
  if (!result.ok) {
    return null;
  }
  let containers = [];
  try {
    containers = JSON.parse(result.stdout);
  } catch {
    return null;
  }
  const candidates = containers
    .filter((container) => container?.status?.state === "running")
    .filter((container) => {
      const id = String(container?.id || "");
      const image = String(container?.configuration?.image?.reference || "");
      return id.includes("finite-agent") || image.includes("finite-agent-runtime");
    })
    .sort((left, right) => String(right?.status?.startedDate || "").localeCompare(String(left?.status?.startedDate || "")));
  return candidates[0]?.id || null;
}

function readContainerHermesNpub(containerId) {
  const code = [
    "import json, pathlib",
    "paths = [pathlib.Path('/tmp/finitechat-invite.json')]",
    "for path in paths:",
    "    if path.exists():",
    "        data = json.loads(path.read_text())",
    "        npub = data.get('npub')",
    "        if npub:",
    "            print(npub)",
    "            raise SystemExit(0)",
    "raise SystemExit(1)",
  ].join("\n");
  const result = command("container", ["exec", containerId, "python3", "-c", code], { check: false });
  return result.ok ? result.stdout.trim() : null;
}

async function finishOnboarding() {
  const choice = page.getByRole("button", { name: /Continue with (this device|imported account)/ });
  if (await choice.isVisible({ timeout: 30_000 }).catch(() => false)) {
    await choice.click();
    step("onboarding_completed");
  } else {
    step("onboarding_not_present");
  }
}

async function connectHermes(target) {
  const input = page.getByPlaceholder("Paste Hermes npub or profile link");
  await input.waitFor({ state: "visible", timeout: 60_000 });
  await input.fill(target);
  await page.getByRole("button", { name: "Connect Hermes" }).click();
  step("hermes_target_submitted");
}

async function waitForComposer() {
  await page.locator("textarea").waitFor({ state: "visible", timeout: 60_000 });
  await page.waitForFunction(
    () => {
      const textarea = document.querySelector("textarea");
      return Boolean(textarea && !textarea.disabled && textarea.placeholder.startsWith("Message "));
    },
    null,
    { timeout: 60_000 }
  );
}

async function appState() {
  return daemonJson("/v1/app/state");
}

async function daemonJson(pathname) {
  const response = await fetch(`${daemonUrl}${pathname}`);
  if (!response.ok) {
    const text = await response.text();
    throw new Error(`${pathname} failed with ${response.status}: ${text}`);
  }
  return response.json();
}

async function waitFor(label, timeoutMs, fn) {
  const started = Date.now();
  let lastError = null;
  while (Date.now() - started < timeoutMs) {
    try {
      const value = await fn();
      if (value) {
        return value;
      }
    } catch (error) {
      lastError = error;
    }
    await delay(500);
  }
  const suffix = lastError ? ` Last error: ${lastError.message}` : "";
  throw new Error(`Timed out waiting for ${label} after ${timeoutMs}ms.${suffix}`);
}

async function freePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : null;
      server.close(() => {
        if (port) {
          resolve(port);
        } else {
          reject(new Error("Could not allocate a local daemon port"));
        }
      });
    });
  });
}

function command(cmd, args, { check = true } = {}) {
  const result = spawnSync(cmd, args, {
    cwd: repoRoot,
    encoding: "utf8",
    maxBuffer: 1024 * 1024 * 8,
  });
  const ok = result.status === 0;
  if (check && !ok) {
    throw new Error(`${cmd} ${args.join(" ")} failed: ${result.stderr || result.stdout}`);
  }
  return {
    ok,
    stdout: result.stdout || "",
    stderr: result.stderr || "",
    status: result.status,
  };
}

async function captureFailureArtifacts() {
  if (!artifactDir) {
    return;
  }
  if (page) {
    try {
      await page.screenshot({ path: path.join(artifactDir, "failure.png"), fullPage: true });
    } catch {
      // Keep the original failure.
    }
  }
  await writeStateSnapshot("failure-state.json");
}

async function writeStateSnapshot(filename) {
  if (!artifactDir || !daemonUrl) {
    return;
  }
  try {
    const state = await appState();
    await writeFile(path.join(artifactDir, filename), `${JSON.stringify(redactState(state), null, 2)}\n`);
  } catch {
    // The daemon may be unavailable on startup failures.
  }
}

function redactState(state) {
  return {
    ...state,
    identity: {
      ...state.identity,
      account_secret_hex: state.identity?.account_secret_hex ? "<redacted>" : undefined,
    },
  };
}

async function writeReport() {
  if (!artifactDir) {
    return;
  }
  await writeFile(path.join(artifactDir, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
}

async function closeApp() {
  if (!electronApp || keepApp) {
    return;
  }
  try {
    await electronApp.close();
  } catch {
    // The app may have already exited.
  }
  await stopSmokeDaemon();
}

async function stopSmokeDaemon() {
  if (!daemonPort) {
    return;
  }
  const pids = pidsListeningOnPort(daemonPort);
  for (const pid of pids) {
    try {
      process.kill(pid, "SIGTERM");
    } catch {
      // Already exited.
    }
  }
  const deadline = Date.now() + 3_000;
  while (Date.now() < deadline && pidsListeningOnPort(daemonPort).length > 0) {
    await delay(100);
  }
  for (const pid of pidsListeningOnPort(daemonPort)) {
    try {
      process.kill(pid, "SIGKILL");
    } catch {
      // Already exited.
    }
  }
}

function pidsListeningOnPort(port) {
  const result = command("lsof", [`-tiTCP:${port}`, "-sTCP:LISTEN"], { check: false });
  if (!result.ok) {
    return [];
  }
  return result.stdout
    .split(/\s+/)
    .map((value) => Number(value))
    .filter((value) => Number.isInteger(value) && value > 0);
}

function npubFromTarget(target) {
  const match = target.match(/npub1[02-9ac-hj-np-z]+/i);
  return match ? match[0] : null;
}

function step(name, facts = {}) {
  report.steps.push({ name, at: new Date().toISOString(), ...facts });
  console.error(`[electron-hermes-smoke] ${name}`);
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
