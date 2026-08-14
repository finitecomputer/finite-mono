import { appendFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(harnessRoot, "../..");
const registryPath = path.join(harnessRoot, "runs/devfinity-state-dirs.jsonl");
const cleanupEnvNames = [
  "DEVFINITY_APPLE_CONTAINER_NAME_PREFIX",
  "DEVFINITY_PORT_OFFSET",
  "DEVFINITY_RUNTIME_AGENT_PORT",
  "DEVFINITY_RUNTIME_IMAGE_REF",
];

const activeStateDirs = new Map();
let handlersInstalled = false;
let processExiting = false;

export function installDevfinityCleanupHandlers({ handleSignals = true, sweepOnStartup = false } = {}) {
  if (sweepOnStartup) {
    sweepTrackedDevfinityStateDirsSync({ reason: "startup" });
  }
  if (!handleSignals) {
    return;
  }
  if (handlersInstalled) {
    return;
  }
  handlersInstalled = true;

  process.once("exit", () => {
    cleanupActiveDevfinityStateDirsSync({ reason: "process exit" });
  });
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.once(signal, () => {
      if (processExiting) {
        return;
      }
      processExiting = true;
      cleanupActiveDevfinityStateDirsSync({ reason: signal });
      process.exit(signalExitCode(signal));
    });
  }
}

export function devfinityStateRoot() {
  return path.resolve(process.env.SKILL_AB_DEVFINITY_STATE_ROOT || path.join(tmpdir(), "finite-skills-ab-devfinity"));
}

export function devfinityCleanupEnv(env = process.env) {
  const cleanupEnv = {};
  for (const name of cleanupEnvNames) {
    if (env[name]) {
      cleanupEnv[name] = env[name];
    }
  }
  return cleanupEnv;
}

export function registerDevfinityStateDir({ artifactDir, cleanupEnv = {}, stateDir, variant }) {
  const absoluteStateDir = path.resolve(stateDir);
  activeStateDirs.set(absoluteStateDir, {
    artifactDir,
    cleanupEnv,
    pid: process.pid,
    stateDir: absoluteStateDir,
    variant,
  });
  appendRegistryEvent({
    artifactDir,
    cleanupEnv,
    event: "track",
    pid: process.pid,
    ppid: process.ppid,
    stateDir: absoluteStateDir,
    timestamp: new Date().toISOString(),
    variant,
  });
  return absoluteStateDir;
}

export function cleanupDevfinityStateDirSync(stateDir, options = {}) {
  const absoluteStateDir = path.resolve(stateDir);
  const entry = activeStateDirs.get(absoluteStateDir) ?? {
    cleanupEnv: options.cleanupEnv ?? {},
    stateDir: absoluteStateDir,
  };
  return cleanupStateDirSync(entry, options);
}

export function cleanupActiveDevfinityStateDirsSync(options = {}) {
  for (const entry of [...activeStateDirs.values()]) {
    cleanupStateDirSync(entry, options);
  }
}

export function sweepTrackedDevfinityStateDirsSync({ includeLive = false, reason = "startup" } = {}) {
  const entries = new Map([...readTrackedStateDirs(), ...discoverTempRootStateDirs()]);
  for (const entry of entries.values()) {
    if (!isHarnessOwnedStateDir(entry.stateDir)) {
      continue;
    }
    if (!includeLive && entry.pid && isProcessAlive(entry.pid)) {
      continue;
    }
    if (entry.cleanedAt && !existsSync(entry.stateDir)) {
      continue;
    }
    cleanupStateDirSync(entry, { reason });
  }
}

function cleanupStateDirSync(entry, { cleanupEnv = entry.cleanupEnv ?? {}, logPath, reason = "cleanup" } = {}) {
  const stateDir = path.resolve(entry.stateDir);
  if (!isHarnessOwnedStateDir(stateDir)) {
    return { ok: true, skipped: true };
  }

  const env = { ...process.env, ...cleanupEnv };
  const invocation = devfinityCleanupInvocation(stateDir);
  const result = spawnSync(invocation.command, invocation.args, {
    cwd: repoRoot,
    encoding: "utf8",
    env,
    timeout: cleanupTimeoutMs(),
  });
  let removed = false;
  let removeError = null;
  try {
    rmSync(stateDir, { recursive: true, force: true });
    removed = true;
  } catch (error) {
    removeError = error;
  }

  activeStateDirs.delete(stateDir);
  appendRegistryEvent({
    event: "cleanup",
    pid: process.pid,
    reason,
    removed,
    removeError: removeError?.message,
    signal: result.signal,
    stateDir,
    status: result.status,
    timestamp: new Date().toISOString(),
  });

  const ok = !result.error && result.status === 0 && !removeError;
  const log = cleanupLog({ invocation, reason, removeError, removed, result, stateDir });
  if (logPath) {
    try {
      appendFileSync(logPath, `${log}\n`, "utf8");
    } catch (error) {
      process.stderr.write(`failed to append Devfinity cleanup log: ${error.message}\n`);
    }
  } else if (!ok) {
    process.stderr.write(`${log}\n`);
  }
  return { ok, removed, status: result.status };
}

function devfinityCleanupInvocation(stateDir) {
  if (process.env.SKILL_AB_DEVFINITY_BIN) {
    return {
      args: ["--state-dir", stateDir, "cleanup"],
      command: process.env.SKILL_AB_DEVFINITY_BIN,
    };
  }
  return {
    args: ["run", "-p", "devfinity", "--", "--state-dir", stateDir, "cleanup"],
    command: process.env.SKILL_AB_DEVFINITY_CARGO_BIN || "cargo",
  };
}

function cleanupLog({ invocation, reason, removeError, removed, result, stateDir }) {
  return [
    "",
    `--- Devfinity cleanup (${reason}) ${stateDir} ---`,
    `$ ${invocation.command} ${invocation.args.map(shellQuote).join(" ")}`,
    result.stdout?.trim(),
    result.stderr?.trim(),
    result.error ? `cleanup error: ${result.error.message}` : `cleanup exit: ${result.signal ?? result.status}`,
    removed ? "state dir removed" : `state dir removal failed: ${removeError?.message ?? "unknown error"}`,
  ]
    .filter(Boolean)
    .join("\n");
}

function cleanupTimeoutMs() {
  const value = Number(process.env.SKILL_AB_DEVFINITY_CLEANUP_TIMEOUT_MS || 120000);
  return Number.isFinite(value) && value > 0 ? value : 120000;
}

function readTrackedStateDirs() {
  if (!existsSync(registryPath)) {
    return new Map();
  }
  const entries = new Map();
  for (const line of readFileSync(registryPath, "utf8").split(/\r?\n/)) {
    if (!line.trim()) {
      continue;
    }
    let event;
    try {
      event = JSON.parse(line);
    } catch {
      continue;
    }
    if (!event.stateDir) {
      continue;
    }
    const stateDir = path.resolve(event.stateDir);
    if (event.event === "track") {
      entries.set(stateDir, { ...event, stateDir });
    } else if (event.event === "cleanup") {
      const current = entries.get(stateDir) ?? { stateDir };
      entries.set(stateDir, { ...current, cleanedAt: event.timestamp });
    }
  }
  return entries;
}

function discoverTempRootStateDirs() {
  const root = devfinityStateRoot();
  const entries = new Map();
  if (!existsSync(root)) {
    return entries;
  }
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.startsWith("skill-")) {
      continue;
    }
    const stateDir = path.join(root, entry.name);
    entries.set(stateDir, { stateDir });
  }
  return entries;
}

function appendRegistryEvent(event) {
  try {
    mkdirSync(path.dirname(registryPath), { recursive: true });
    appendFileSync(registryPath, `${JSON.stringify(event)}\n`, "utf8");
  } catch (error) {
    process.stderr.write(`failed to update Devfinity state-dir registry: ${error.message}\n`);
  }
}

function isHarnessOwnedStateDir(stateDir) {
  const absoluteStateDir = path.resolve(stateDir);
  const root = devfinityStateRoot();
  const relative = path.relative(root, absoluteStateDir);
  if (relative && !relative.startsWith("..") && !path.isAbsolute(relative)) {
    return path.basename(absoluteStateDir).startsWith("skill-");
  }
  return path.basename(path.dirname(absoluteStateDir)) === "finite-skills-ab-devfinity" && path.basename(absoluteStateDir).startsWith("skill-");
}

function isProcessAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) {
    return false;
  }
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function signalExitCode(signal) {
  return {
    SIGHUP: 129,
    SIGINT: 130,
    SIGTERM: 143,
  }[signal] ?? 1;
}

function shellQuote(value) {
  const text = String(value);
  if (/^[A-Za-z0-9_/:=.,@+-]+$/.test(text)) {
    return text;
  }
  return `'${text.replaceAll("'", "'\"'\"'")}'`;
}
