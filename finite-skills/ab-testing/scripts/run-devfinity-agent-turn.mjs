#!/usr/bin/env node
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(harnessRoot, "../..");

const promptFile = requiredEnv("SKILL_AB_DEVFINITY_PROMPT_FILE");
const resultFile = requiredEnv("SKILL_AB_DEVFINITY_RESULT_FILE");
const skillBundleDir = requiredEnv("SKILL_AB_DEVFINITY_SKILL_BUNDLE_DIR");
const runtimeOutputPath = requiredEnv("SKILL_AB_DEVFINITY_RUNTIME_OUTPUT_PATH");
const coreUrl = trimTrailingSlash(requiredEnv("FC_CORE_URL"));
const dashboardUrl = trimTrailingSlash(requiredEnv("FC_DASHBOARD_URL"));
const dashboardOrigin = dashboardUrl.replace(/\/dashboard\/?$/u, "");
const stateDir = requiredEnv("DEVFINITY_STATE_DIR");
const profile = requiredEnv("DEVFINITY_PROFILE");
const prompt = readFileSync(promptFile, "utf8");
const variant = process.env.SKILL_AB_DEVFINITY_VARIANT || "skill";
const replyTimeoutMs = positiveIntegerEnv("SKILL_AB_DEVFINITY_REPLY_TIMEOUT_MS", 20 * 60 * 1000);

let updatesProcess = null;

try {
  const driver = runtimeDriver(profile);
  const runtime = await ensureRuntime();
  const containerId = await runtimeContainerId(driver, runtime.projectId);
  await waitHttp("Agent Runtime", `${runtime.runtimeUrl}/healthz`, 180000);
  await installSkillBundle(driver, containerId);
  await restartHermes(driver, containerId);
  await waitHttp("Agent Runtime after Hermes restart", `${runtime.runtimeUrl}/healthz`, 180000);
  startChatUpdates(runtime.machineId);
  await waitForConnectedChat(runtime.machineId);
  await createTopic(runtime.machineId, `Local web build ${Date.now()}`);
  const finalReply = await sendAndCaptureReply(runtime.machineId, prompt);
  const html = await readRuntimeFile(driver, containerId, runtimeOutputPath).catch(() => finalReply);

  mkdirSync(path.dirname(resultFile), { recursive: true });
  writeFileSync(
    resultFile,
    JSON.stringify(
      {
        containerId,
        finalReply,
        html,
        runtimeOutputPath,
        variant,
      },
      null,
      2,
    ),
    "utf8",
  );
} finally {
  if (updatesProcess) {
    updatesProcess.kill("SIGTERM");
  }
}

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is required`);
  }
  return value;
}

function runtimeDriver(value) {
  if (value === "apple-saas") {
    return "apple";
  }
  if (value === "docker-saas") {
    return "docker";
  }
  throw new Error(`Devfinity agent turn requires apple-saas or docker-saas, got ${value}`);
}

async function ensureRuntime() {
  let runtime = runtimeFromMe(await readCoreMe()).runtime;
  if (runtime) {
    return runtime;
  }

  const me = await readCoreMe();
  const pending = (me.agent_creation_requests || []).some((request) =>
    ["requested", "launching"].includes(request.status),
  );
  if (!pending) {
    await submitAgentCreationRequest();
  }

  const started = Date.now();
  while (Date.now() - started < 600000) {
    const parsed = runtimeFromMe(await readCoreMe());
    if (parsed.runtime) {
      return parsed.runtime;
    }
    if (parsed.failed) {
      throw new Error(parsed.failed);
    }
    await delay(2000);
  }
  throw new Error("Core did not report a launched Devfinity runtime within 600s");
}

async function readCoreMe() {
  const tokenPath = path.join(stateDir, "workos-fixture/dashboard-customer.jwt");
  const token = readFileSync(tokenPath, "utf8").trim();
  return await fetchJson(`${coreUrl}/api/core/v1/me`, {
    headers: { authorization: `Bearer ${token}` },
  });
}

function runtimeFromMe(me) {
  const visible = (me.projects || []).find((entry) => entry.runtime);
  if (visible) {
    const project = visible.project;
    const runtime = visible.runtime;
    const contact = new URL(runtime.contact_endpoint);
    contact.pathname = contact.pathname.replace(/\/contact\/?$/u, "");
    return {
      runtime: {
        agentEmail: project.agent_email || "",
        machineId: runtime.id,
        projectId: project.id,
        runtimeUrl: contact.toString().replace(/\/$/u, ""),
      },
    };
  }
  const failed = (me.agent_creation_requests || []).find((request) => request.status === "failed");
  return {
    failed: failed ? `Agent launch failed: ${failed.failure_message || failed.id}` : null,
    runtime: null,
  };
}

async function submitAgentCreationRequest() {
  const operatorTokenPath = path.join(stateDir, "workos-fixture/operator.jwt");
  const operatorToken = readFileSync(operatorTokenPath, "utf8").trim();
  const issued = await fetchJson(`${coreUrl}/api/core/v1/admin/launch-code-batches`, {
    body: JSON.stringify({
      codeCount: 1,
      expiresInHours: 24,
      name: "Local web build",
    }),
    headers: {
      authorization: `Bearer ${operatorToken}`,
      "content-type": "application/json",
    },
    method: "POST",
  });
  const launchCode = issued?.codes?.[0]?.code;
  if (!launchCode) {
    throw new Error("Core did not return a Devfinity launch code");
  }

  const form = new URLSearchParams();
  form.set("displayName", `Local web build ${Date.now()}`);
  form.set("hostingTier", "standard");
  form.set("access", "launch-code");
  form.set("launchCode", launchCode);
  form.set("idempotencyKey", `skill-ab-${variant}-${Date.now()}`);

  await fetchText(`${dashboardUrl}/agent-creation-requests`, {
    body: form,
    method: "POST",
  });
}

async function runtimeContainerId(driver, projectId) {
  if (driver === "docker") {
    const output = await runCapture("docker", [
      "ps",
      "--format",
      "{{json .}}",
      "--filter",
      `label=computer.finite.v2.project_id=${projectId}`,
    ]);
    const rows = output
      .trim()
      .split(/\n+/u)
      .filter(Boolean)
      .map((line) => JSON.parse(line));
    if (rows.length !== 1 || !rows[0].ID) {
      throw new Error(`Expected one Docker runtime for ${projectId}, found ${rows.length}`);
    }
    return rows[0].ID;
  }

  const output = await runCapture("container", ["list", "--format", "json"]);
  const containers = JSON.parse(output);
  const runtime = containers.find(
    (entry) => entry?.configuration?.labels?.["computer.finite.v2.project_id"] === projectId,
  );
  if (!runtime?.configuration?.id) {
    throw new Error(`Could not find Apple Container runtime for ${projectId}`);
  }
  return runtime.configuration.id;
}

async function installSkillBundle(driver, containerId) {
  if (!existsSync(skillBundleDir)) {
    throw new Error(`Skill bundle directory does not exist: ${skillBundleDir}`);
  }
  const installScript = String.raw`
set -euo pipefail
root="/data/agent/managed-skills/finite"
current="$root/current"
staging="$root/.ab-staging-$(date +%s%N)-$$"
previous="$root/.ab-previous"
rm -rf "$staging"
mkdir -p "$staging"
tar -C "$staging" -xf -
test "$(find "$staging" -name SKILL.md -type f | wc -l | tr -d ' ')" = "1"
rm -rf "$previous"
if test -e "$current"; then
  mv "$current" "$previous"
fi
mv "$staging" "$current"
find "$current" -name SKILL.md -type f -print
`;
  await runTarIntoRuntime(driver, containerId, skillBundleDir, installScript);
}

async function restartHermes(driver, containerId) {
  const script = String.raw`
set -euo pipefail
read_status() {
  finite-agentd status --json | python3 -c 'import json,sys; d=json.load(sys.stdin); h=d["processes"]["processes"].get("hermes", {}); print("{}\t{}\t{}".format(h.get("pid") or "", h.get("restart_count") or 0, h.get("state") or ""))'
}
before="$(read_status)"
old_pid="$(printf "%s" "$before" | cut -f1)"
old_restart="$(printf "%s" "$before" | cut -f2)"
if test -n "$old_pid"; then
  kill "$old_pid" 2>/dev/null || true
fi
for _ in $(seq 1 300); do
  current="$(read_status)"
  pid="$(printf "%s" "$current" | cut -f1)"
  restart="$(printf "%s" "$current" | cut -f2)"
  state="$(printf "%s" "$current" | cut -f3)"
  if test "$state" = "running" && test -n "$pid" && test "$pid" != "$old_pid" && test "$restart" -gt "$old_restart"; then
    exit 0
  fi
  sleep 0.2
done
echo "Hermes did not restart after managed-skills replacement" >&2
exit 1
`;
  await runtimeExec(driver, containerId, ["/bin/bash", "-lc", script]);
}

function startChatUpdates(machineId) {
  updatesProcess = spawn(
    "curl",
    ["-fsS", "--no-buffer", `${dashboardOrigin}/api/chat/machines/${machineId}/hosted-device/updates`],
    { cwd: repoRoot, stdio: ["ignore", "ignore", "ignore"] },
  );
}

async function waitForConnectedChat(machineId) {
  const started = Date.now();
  while (Date.now() - started < 180000) {
    const state = await chatState(machineId).catch(() => null);
    if ((state?.rooms || []).some((room) => room.is_agent_chat && room.state === "Connected")) {
      return;
    }
    await delay(2000);
  }
  throw new Error("Hosted Web Device did not establish the agent chat within 180s");
}

async function createTopic(machineId, title) {
  const state = await chatState(machineId);
  const roomId = state.hosted_agent_binding?.canonical_room_id;
  if (!roomId) {
    throw new Error("Canonical agent room is unavailable");
  }
  const next = await chatAction(machineId, { CreateTopic: { room_id: roomId, title } });
  const topic = (next.topics || []).find(
    (candidate) =>
      candidate.room_id === roomId &&
      candidate.topic_id === next.selected_topic_id &&
      candidate.title === title,
  );
  const chat = topic?.chats?.find((candidate) => candidate.chat_id === next.selected_chat_id);
  if (!topic || !chat) {
    throw new Error("CreateTopic did not select its canonical first chat");
  }
}

async function sendAndCaptureReply(machineId, text) {
  let state = await chatState(machineId);
  const route = selectedChatRoute(state);
  await waitForScopedWorkingClear(machineId, route);
  state = await chatState(machineId);
  const before = maxRemoteSeq(state, route);
  await chatAction(machineId, {
    SendChatMessage: {
      chat_id: route.chatId,
      room_id: route.roomId,
      text,
      topic_id: route.topicId,
    },
  });
  const finalState = await waitForRemoteReplyAfter(machineId, before, route);
  const final = (finalState.messages || []).findLast(
    (message) =>
      message.sender_account_id !== finalState.identity?.account_id &&
      Number(message.seq) > before &&
      message.room_id === route.roomId &&
      message.conversation_id === route.topicId &&
      message.chat_id === route.chatId &&
      message.final_delivery === true,
  );
  if (!final) {
    throw new Error("Hermes turn has no scoped final delivery");
  }
  return String(final.display_content || final.text || "");
}

function selectedChatRoute(state) {
  const roomId = state.hosted_agent_binding?.canonical_room_id;
  const room = (state.rooms || []).find((candidate) => candidate.room_id === roomId && candidate.state === "Connected");
  const topics = (state.topics || []).filter((candidate) => candidate.room_id === room?.room_id && !candidate.archived);
  const topic =
    topics.find((candidate) => candidate.topic_id === state.selected_topic_id) ||
    topics.find((candidate) => candidate.topic_id === "home");
  const chat =
    topic?.chats?.find((candidate) => candidate.chat_id === state.selected_chat_id) ||
    topic?.chats?.find((candidate) => candidate.active) ||
    topic?.chats?.[0];
  if (!room || !topic || !chat) {
    throw new Error("Canonical Home chat is unavailable");
  }
  return { chatId: chat.chat_id, roomId: room.room_id, topicId: topic.topic_id };
}

function maxRemoteSeq(state, route) {
  return (state.messages || [])
    .filter(
      (message) =>
        message.sender_account_id !== state.identity?.account_id &&
        message.room_id === route.roomId &&
        message.conversation_id === route.topicId &&
        message.chat_id === route.chatId,
    )
    .reduce((max, message) => Math.max(max, Number(message.seq) || 0), 0);
}

async function waitForRemoteReplyAfter(machineId, previous, route) {
  const started = Date.now();
  while (Date.now() - started < replyTimeoutMs) {
    const state = await chatState(machineId);
    const hasFinal = (state.messages || []).some(
      (message) =>
        message.sender_account_id !== state.identity?.account_id &&
        Number(message.seq) > previous &&
        message.room_id === route.roomId &&
        message.conversation_id === route.topicId &&
        message.chat_id === route.chatId &&
        message.final_delivery === true,
    );
    if (hasFinal) {
      await waitForScopedWorkingClear(machineId, route);
      return state;
    }
    await delay(500);
  }
  throw new Error(`Hermes did not answer the Devfinity skill A/B message within ${Math.round(replyTimeoutMs / 1000)}s`);
}

async function waitForScopedWorkingClear(machineId, route) {
  const started = Date.now();
  while (Date.now() - started < 30000) {
    const state = await chatState(machineId);
    const present = (state.typing_members || []).some(
      (member) =>
        member.room_id === route.roomId &&
        member.topic_id === route.topicId &&
        member.chat_id === route.chatId &&
        member.activity_kind === "working",
    );
    if (!present) {
      return;
    }
    await delay(250);
  }
  throw new Error("Scoped Working activity did not clear");
}

async function chatState(machineId) {
  return await fetchJson(`${dashboardOrigin}/api/chat/machines/${machineId}/hosted-device/state`, {
    timeoutMs: 30000,
  });
}

async function chatAction(machineId, action) {
  return await fetchJson(`${dashboardOrigin}/api/chat/machines/${machineId}/hosted-device/actions`, {
    body: JSON.stringify(action),
    headers: { "content-type": "application/json" },
    method: "POST",
    timeoutMs: 30000,
  });
}

async function readRuntimeFile(driver, containerId, filePath) {
  return await runtimeExec(driver, containerId, ["cat", filePath]);
}

async function waitHttp(name, url, timeoutMs) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    try {
      await fetchText(url, { timeoutMs: 3000 });
      return;
    } catch {
      await delay(1000);
    }
  }
  throw new Error(`${name} did not become ready at ${url}`);
}

async function fetchJson(url, options = {}) {
  const text = await fetchText(url, options);
  return JSON.parse(text);
}

async function fetchText(url, options = {}) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), options.timeoutMs || 30000);
  try {
    const response = await fetch(url, {
      body: options.body,
      headers: options.headers,
      method: options.method || "GET",
      signal: controller.signal,
    });
    const text = await response.text();
    if (!response.ok) {
      throw new Error(`${options.method || "GET"} ${url} failed with ${response.status}: ${text.slice(0, 500)}`);
    }
    return text;
  } finally {
    clearTimeout(timeout);
  }
}

async function runTarIntoRuntime(driver, containerId, sourceDir, script) {
  const tar = spawn("tar", ["-C", sourceDir, "-cf", "-", "."], {
    cwd: repoRoot,
    stdio: ["ignore", "pipe", "pipe"],
  });
  const exec = spawn(runtimeCommand(driver), [...runtimeExecPrefix(driver, containerId, true), "/bin/bash", "-lc", script], {
    cwd: repoRoot,
    stdio: ["pipe", "pipe", "pipe"],
  });
  tar.stdout.pipe(exec.stdin);
  const [tarResult, execResult] = await Promise.all([waitForProcess(tar), waitForProcess(exec)]);
  if (tarResult.code !== 0) {
    throw new Error(`tar exited with ${tarResult.code}: ${tarResult.stderr}`);
  }
  if (execResult.code !== 0) {
    throw new Error(`${runtimeCommand(driver)} exec exited with ${execResult.code}: ${execResult.stderr || execResult.stdout}`);
  }
}

async function runtimeExec(driver, containerId, args) {
  return await runCapture(runtimeCommand(driver), [...runtimeExecPrefix(driver, containerId, false), ...args]);
}

function runtimeCommand(driver) {
  return driver === "docker" ? "docker" : "container";
}

function runtimeExecPrefix(driver, containerId, interactive) {
  if (driver === "docker") {
    return interactive ? ["exec", "-i", containerId] : ["exec", containerId];
  }
  return interactive ? ["exec", "-i", containerId] : ["exec", containerId];
}

async function runCapture(command, args) {
  const child = spawn(command, args, {
    cwd: repoRoot,
    stdio: ["ignore", "pipe", "pipe"],
  });
  const result = await waitForProcess(child);
  if (result.code !== 0) {
    throw new Error(`${command} ${args.join(" ")} exited with ${result.code}: ${result.stderr || result.stdout}`);
  }
  return result.stdout;
}

async function waitForProcess(child) {
  const stdout = [];
  const stderr = [];
  child.stdout?.on("data", (chunk) => stdout.push(chunk));
  child.stderr?.on("data", (chunk) => stderr.push(chunk));
  return await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", (code, signal) => {
      resolve({
        code: code ?? (signal ? 1 : 0),
        stderr: Buffer.concat(stderr).toString("utf8"),
        stdout: Buffer.concat(stdout).toString("utf8"),
      });
    });
  });
}

function trimTrailingSlash(value) {
  return String(value).replace(/\/+$/u, "");
}

function positiveIntegerEnv(name, fallback) {
  const raw = process.env[name];
  if (!raw) {
    return fallback;
  }
  const parsed = Number(raw);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`${name} must be a positive integer number of milliseconds`);
  }
  return parsed;
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
