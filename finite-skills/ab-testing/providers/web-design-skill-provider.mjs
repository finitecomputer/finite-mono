import { spawn, spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  cleanupDevfinityStateDirSync,
  devfinityCleanupEnv,
  installDevfinityCleanupHandlers,
  registerDevfinityStateDir,
} from "../scripts/devfinity-cleanup.mjs";

const providerDir = path.dirname(fileURLToPath(import.meta.url));
const harnessRoot = path.resolve(providerDir, "..");
const repoRoot = path.resolve(harnessRoot, "../..");
const finitePrivateDefaultBaseUrl = "https://kimi-k2-6.finite.containers.tinfoil.dev/v1";
const finitePrivateDefaultModel = "glm-5-2";
const openAIDefaultBaseUrl = "https://api.openai.com/v1";
const openAIDefaultModel = "gpt-5-mini";

installDevfinityCleanupHandlers({ sweepOnStartup: true });

export default class WebDesignSkillProvider {
  constructor(options = {}) {
    this.config = options.config ?? {};
    this.providerId = options.id ?? "web-design-skill-provider";
    this.variant = this.config.variant ?? options.label ?? "skill";
  }

  id() {
    return this.variant;
  }

  async callApi(prompt, context = {}) {
    const vars = context.vars ?? {};
    const caseId = sanitize(vars.caseId || vars.title || "case");
    const skillPath = this.resolveSkillPath();
    const skillSourcePath = this.resolveSkillSourcePath(skillPath);
    const skillText = readFileSync(skillPath, "utf8");
    const skillName = parseSkillName(skillText) ?? path.basename(path.dirname(skillPath));
    const artifactDir = path.resolve(harnessRoot, this.config.outputDir ?? "./runs/latest/artifacts", caseId, sanitize(this.variant));
    mkdirSync(artifactDir, { recursive: true });

    const directProviderPrompt = buildAgentPrompt({
      brief: prompt,
      caseTitle: vars.title ?? vars.caseId ?? caseId,
      skillName,
      skillPath,
      skillText,
    });
    const codexAgentPrompt = buildCodexAgentPrompt({
      brief: prompt,
      caseTitle: vars.title ?? vars.caseId ?? caseId,
    });
    const finiteAgentPrompt = buildFiniteAgentPrompt({
      brief: prompt,
      caseTitle: vars.title ?? vars.caseId ?? caseId,
      runId: caseId,
    });

    const startedAt = new Date().toISOString();
    let output;
    let modelProvider;
    let model;
    let promptTranscript = directProviderPrompt.transcript;
    const runner = resolveRunner(this.config);
    let tokenUsage;

    if (runner === "agent") {
      model = process.env.SKILL_AB_AGENT_MODEL || "codex-default";
      modelProvider = { name: "codex-agent", baseUrl: null, keySource: null, label: "Codex isolated agent" };
      const response = await callCodexAgent({
        artifactDir,
        prompt: codexAgentPrompt,
        skillPath,
        timeoutMs: Number(process.env.SKILL_AB_AGENT_TIMEOUT_MS || process.env.SKILL_AB_TIMEOUT_MS || this.config.timeoutMs || 600000),
      });
      output = response.output;
      promptTranscript = response.prompt;
      tokenUsage = {};
    } else if (runner === "devfinity") {
      const devfinityMode = resolveDevfinityMode(this.config);
      const key = devfinityMode === "disposable" ? readFinitePrivateKey({ required: true }) : readFinitePrivateKey({ required: false });
      model = "devfinity-local-hermes";
      modelProvider = {
        name: "devfinity-local-agent",
        baseUrl: null,
        keySource: key?.keySource ?? "already-running devfinity stack",
        label: devfinityMode === "disposable" ? "Devfinity disposable Finite agent" : "Devfinity local Finite agent client",
      };
      const agentRunConfig = {
        artifactDir,
        prompt: finiteAgentPrompt.prompt,
        runtimeOutputPath: finiteAgentPrompt.runtimeOutputPath,
        skillName,
        skillPath,
        skillSourcePath,
        skillText,
        timeoutMs: Number(process.env.SKILL_AB_DEVFINITY_TIMEOUT_MS || this.config.devfinityTimeoutMs || 1800000),
        variant: this.variant,
      };
      const response =
        devfinityMode === "disposable"
          ? await callDisposableDevfinityAgent({ ...agentRunConfig, upstreamKey: key.apiKey })
          : await callDevfinityAgent(agentRunConfig);
      output = response.output;
      promptTranscript = response.prompt;
      tokenUsage = {};
    } else {
      modelProvider = resolveModelProvider(this.config);
      model = modelProvider.model;
      const response = await callResponsesApi({
        apiKey: modelProvider.apiKey,
        baseUrl: modelProvider.baseUrl,
        messages: directProviderPrompt.messages,
        model,
        maxOutputTokens: Number(process.env.SKILL_AB_MAX_OUTPUT_TOKENS || this.config.maxOutputTokens || 6000),
        timeoutMs: Number(process.env.SKILL_AB_TIMEOUT_MS || this.config.timeoutMs || 120000),
        providerLabel: modelProvider.label,
      });
      output = response.output;
      tokenUsage = response.tokenUsage;
    }

    let extraction = extractHtml(output);
    let outputKind = extraction.found ? "html" : "non-html";
    if (runner === "provider" && !extraction.found && shouldRepairHtml(this.config)) {
      const repair = await callResponsesApi({
        apiKey: modelProvider.apiKey,
        baseUrl: modelProvider.baseUrl,
        messages: buildHtmlRepairMessages({ agentOutput: output, brief: prompt, caseTitle: vars.title ?? caseId }),
        model,
        maxOutputTokens: Number(process.env.SKILL_AB_REPAIR_MAX_OUTPUT_TOKENS || process.env.SKILL_AB_MAX_OUTPUT_TOKENS || this.config.maxOutputTokens || 6000),
        timeoutMs: Number(process.env.SKILL_AB_TIMEOUT_MS || this.config.timeoutMs || 120000),
        providerLabel: modelProvider.label,
      });
      const repaired = extractHtml(repair.output);
      output = `${output}\n\n--- HTML repair pass output ---\n\n${repair.output}`;
      tokenUsage = addTokenUsage(tokenUsage, repair.tokenUsage);
      if (repaired.found) {
        extraction = repaired;
        outputKind = "repaired-html";
      }
    }
    if (!extraction.found) {
      const message = `${runner} runner returned non-HTML output; no preview artifact was generated.`;
      writeFileSync(path.join(artifactDir, "output.raw.md"), output, "utf8");
      writeFileSync(path.join(artifactDir, "prompt.txt"), promptTranscript, "utf8");
      writeFileSync(
        path.join(artifactDir, "metadata.json"),
        JSON.stringify(
          {
            caseId,
            caseTitle: vars.title ?? caseId,
            brief: prompt,
            model,
            modelProvider: modelProvider.name,
            baseUrl: modelProvider.baseUrl,
            keySource: modelProvider.keySource,
            outputKind,
            runner,
            skillName,
            skillPath: path.relative(harnessRoot, skillPath),
            startedAt,
            finishedAt: new Date().toISOString(),
            variant: this.variant,
            error: message,
          },
          null,
          2,
        ),
        "utf8",
      );
      throw new Error(`${message} Raw output: ${truncateForError(output)}`);
    }

    const html = extraction.html;
    writeFileSync(path.join(artifactDir, "index.html"), html, "utf8");
    writeFileSync(path.join(artifactDir, "output.raw.md"), output, "utf8");
    writeFileSync(path.join(artifactDir, "prompt.txt"), promptTranscript, "utf8");
    writeFileSync(
      path.join(artifactDir, "metadata.json"),
      JSON.stringify(
        {
          caseId,
          caseTitle: vars.title ?? caseId,
          brief: prompt,
          model,
          modelProvider: modelProvider.name,
          baseUrl: modelProvider.baseUrl,
          keySource: modelProvider.keySource,
          outputKind,
          runner,
          skillName,
          skillPath: path.relative(harnessRoot, skillPath),
          startedAt,
          finishedAt: new Date().toISOString(),
          variant: this.variant,
        },
        null,
        2,
      ),
      "utf8",
    );

    return {
      output: html,
      prompt: promptTranscript,
      tokenUsage,
      metadata: {
        artifactPath: path.relative(harnessRoot, path.join(artifactDir, "index.html")),
        caseId,
        skillName,
        variant: this.variant,
      },
    };
  }

  resolveSkillPath() {
    const envName = `SKILL_AB_${this.variant.toUpperCase().replace(/[^A-Z0-9]+/g, "_")}_PATH`;
    const configured = process.env[envName] || this.config.skillPath;
    if (!configured) {
      throw new Error(`No skillPath configured for ${this.variant}`);
    }
    if (path.isAbsolute(configured)) {
      return configured;
    }
    return path.resolve(harnessRoot, configured);
  }

  resolveSkillSourcePath(fallbackPath) {
    const envName = `SKILL_AB_${this.variant.toUpperCase().replace(/[^A-Z0-9]+/g, "_")}_SOURCE_PATH`;
    const configured = process.env[envName] || this.config.skillSourcePath;
    if (!configured) {
      return fallbackPath;
    }
    if (path.isAbsolute(configured)) {
      return configured;
    }
    const repoCandidate = path.resolve(repoRoot, configured);
    if (existsSync(repoCandidate)) {
      return repoCandidate;
    }
    return path.resolve(harnessRoot, configured);
  }
}

function resolveModelProvider(config = {}) {
  const requestedProvider = normalizeProviderName(process.env.SKILL_AB_PROVIDER || config.provider || "auto");
  if (requestedProvider === "finite-private") {
    return finitePrivateProvider(config);
  }
  if (requestedProvider === "openai") {
    return openAIProvider(config);
  }
  if (requestedProvider !== "auto") {
    throw new Error(`Unsupported SKILL_AB_PROVIDER "${requestedProvider}". Use "auto", "finite-private", or "openai".`);
  }

  const finitePrivateKey = readFinitePrivateKey({ required: false });
  if (finitePrivateKey) {
    return finitePrivateProvider(config, finitePrivateKey);
  }
  if (process.env.OPENAI_API_KEY) {
    return openAIProvider(config);
  }

  throw new Error(
    [
      "A real skill A/B run needs a model API key.",
      "Run `just dev inference-key` to cache a Finite Private key locally, set `FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY`,",
      "or use `SKILL_AB_PROVIDER=openai OPENAI_API_KEY=... pnpm run ab`.",
      "The browser Generate flow always runs the real Devfinity local Finite agent path.",
    ].join(" "),
  );
}

function finitePrivateProvider(config = {}, preloadedKey = null) {
  const key = preloadedKey ?? readFinitePrivateKey({ required: true });
  return {
    name: "finite-private",
    label: "Finite Private",
    apiKey: key.apiKey,
    keySource: key.keySource,
    baseUrl: trimTrailingSlash(
      process.env.SKILL_AB_FINITE_PRIVATE_BASE_URL ||
        process.env.FINITE_PRIVATE_BASE_URL ||
        process.env.FC_RUNNER_FINITE_PRIVATE_BASE_URL ||
        config.finitePrivateBaseUrl ||
        config.baseUrl ||
        finitePrivateDefaultBaseUrl,
    ),
    model:
      process.env.SKILL_AB_MODEL ||
      process.env.FINITE_PRIVATE_MODEL ||
      process.env.FC_RUNNER_FINITE_PRIVATE_MODEL ||
      config.model ||
      finitePrivateDefaultModel,
  };
}

function openAIProvider(config = {}) {
  const apiKey = process.env.OPENAI_API_KEY;
  if (!apiKey) {
    throw new Error(
      "OPENAI_API_KEY is required when SKILL_AB_PROVIDER=openai. For the product runner, run `just dev inference-key` from the repo root to cache the local Finite Private upstream key.",
    );
  }
  return {
    name: "openai",
    label: "OpenAI",
    apiKey,
    keySource: "env:OPENAI_API_KEY",
    baseUrl: trimTrailingSlash(process.env.OPENAI_BASE_URL || config.openAIBaseUrl || config.baseUrl || openAIDefaultBaseUrl),
    model: process.env.SKILL_AB_MODEL || config.model || openAIDefaultModel,
  };
}

function readFinitePrivateKey({ required }) {
  const envCandidates = [
    ["SKILL_AB_FINITE_PRIVATE_KEY", process.env.SKILL_AB_FINITE_PRIVATE_KEY],
    ["FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY", process.env.FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY],
    ["FINITE_PRIVATE_API_KEY", process.env.FINITE_PRIVATE_API_KEY],
  ];
  for (const [name, value] of envCandidates) {
    if (value?.trim()) {
      return validateFinitePrivateKey(value, `env:${name}`);
    }
  }

  const keyFileCandidates = [
    process.env.SKILL_AB_FINITE_PRIVATE_KEY_FILE,
    process.env.DEVFINITY_STATE_DIR
      ? path.join(resolveRepoRelativePath(process.env.DEVFINITY_STATE_DIR), "credentials", "finite-private-upstream.key")
      : null,
    path.join(repoRoot, ".local-state/devfinity/credentials/finite-private-upstream.key"),
    ...gitWorktreeFinitePrivateKeyFiles(),
  ].filter(Boolean);
  for (const keyFile of unique(keyFileCandidates)) {
    const absolutePath = path.isAbsolute(keyFile) ? keyFile : path.resolve(repoRoot, keyFile);
    if (existsSync(absolutePath)) {
      return validateFinitePrivateKey(readFileSync(absolutePath, "utf8"), `file:${displayPath(absolutePath)}`);
    }
  }

  if (required) {
    throw new Error(
      [
        "Finite Private runs require a local upstream key.",
        "Run `just dev inference-key`, set `FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY`,",
        "or set `SKILL_AB_FINITE_PRIVATE_KEY_FILE` to a local key file.",
      ].join(" "),
    );
  }
  return null;
}

function gitWorktreeFinitePrivateKeyFiles() {
  const result = spawnSync("git", ["worktree", "list", "--porcelain"], {
    cwd: repoRoot,
    encoding: "utf8",
    timeout: 5000,
  });
  if (result.status !== 0 || !result.stdout) {
    return [];
  }
  return result.stdout
    .split("\n")
    .filter((line) => line.startsWith("worktree "))
    .map((line) => line.slice("worktree ".length).trim())
    .filter(Boolean)
    .map((worktreePath) => path.join(worktreePath, ".local-state/devfinity/credentials/finite-private-upstream.key"));
}

function unique(values) {
  return [...new Set(values)];
}

function validateFinitePrivateKey(value, keySource) {
  const apiKey = String(value ?? "").trim();
  if (!/^fpk_live_[0-9a-fA-F]{64}$/.test(apiKey)) {
    throw new Error(`Finite Private API key from ${keySource} is not in the expected fpk_live_ format.`);
  }
  return { apiKey, keySource };
}

function normalizeProviderName(value) {
  return String(value ?? "auto").trim().toLowerCase();
}

function resolveRunner(config = {}) {
  const runner = String(process.env.SKILL_AB_RUNNER || config.runner || "devfinity").trim().toLowerCase();
  if (runner === "devfinity" || runner === "agent" || runner === "provider") {
    return runner;
  }
  throw new Error(`Unsupported SKILL_AB_RUNNER "${runner}". Use "devfinity", "agent", or "provider".`);
}

function resolveDevfinityMode(config = {}) {
  const mode = String(process.env.SKILL_AB_DEVFINITY_MODE || config.devfinityMode || "client").trim().toLowerCase();
  if (["client", "existing", "reuse", "reusable", "server-client"].includes(mode)) {
    return "client";
  }
  if (["disposable", "isolated", "legacy"].includes(mode)) {
    return "disposable";
  }
  throw new Error(`Unsupported SKILL_AB_DEVFINITY_MODE "${mode}". Use "client" or "disposable".`);
}

function trimTrailingSlash(value) {
  return String(value).replace(/\/+$/g, "");
}

function resolveRepoRelativePath(filePath) {
  return path.isAbsolute(filePath) ? filePath : path.resolve(repoRoot, filePath);
}

function displayPath(filePath) {
  const relative = path.relative(repoRoot, filePath);
  if (relative && !relative.startsWith("..") && !path.isAbsolute(relative)) {
    return relative;
  }
  return filePath;
}

function buildAgentPrompt({ brief, caseTitle, skillName, skillPath, skillText }) {
  const system = `You are an autonomous web design implementation agent.

You have exactly one local agent skill installed for this run. Use the installed
skill silently as your operating guidance for design quality, implementation
choices, verification standards, and tradeoffs. Do not summarize, critique,
quote, compare, or explain the skill.

Installed skill: ${skillName}
Installed skill path: ${skillPath}

<skill>
${skillText}
</skill>

Your response is consumed by an automated visual review harness. The only valid
response is a complete self-contained HTML document. The first bytes of your
response must be <!doctype html>.`;

  const user = `Build this web artifact:

${brief}

Delivery requirements:
- Return only a complete single-file HTML document.
- Inline all CSS and JavaScript.
- Do not fetch external fonts, images, scripts, or stylesheets.
- Use realistic copy, data, controls, and visual states.
- Make the first viewport useful for judging the design.
- Do not mention the skill, the test harness, variants, or this prompt inside the rendered UI.
- Keep the artifact safe to open from a local file URL.

Case title: ${caseTitle}`;

  return {
    messages: [
      { role: "system", content: system },
      { role: "user", content: user },
    ],
    transcript: `<system>
${system}
</system>

<user>
${user}
</user>
`,
  };
}

function buildCodexAgentPrompt({ brief, caseTitle }) {
  return `Use your configured local skill as your operating guidance for this task.
Do not ask clarifying questions.

Build this web artifact:

${brief}

Write the result to output/index.html in this workspace.

Hard requirements:
- Create a complete single-file HTML document at output/index.html.
- Inline all CSS and JavaScript.
- Do not fetch external fonts, images, scripts, or stylesheets.
- Use realistic copy, data, controls, and visual states.
- Make the first viewport useful for judging the design.
- Keep the artifact safe to open from a local file URL.

Case title: ${caseTitle}

When finished, reply with a short confirmation only.`;
}

function buildFiniteAgentPrompt({ brief, caseTitle, runId }) {
  const runtimeOutputPath = `/data/workspace/local-web-builds/${runId}/index.html`;
  return {
    prompt: `Use your configured local skill as design guidance for this task.
Do not ask clarifying questions.

Build this web artifact:

${brief}

Return exactly one complete single-file HTML document in your final response.
Start with <!doctype html>. Do not wrap the HTML in a Markdown code fence.

Hard requirements:
- Inline all CSS and JavaScript.
- Do not fetch external fonts, images, scripts, or stylesheets.
- Use realistic copy, data, controls, and visual states.
- Make the first viewport useful for judging the design.
- Keep the artifact safe to open from a local file URL.
- Do not mention these delivery instructions in the rendered UI.
- Do not run deployment, publishing, browser automation, or extended QA flows.

Case title: ${caseTitle}

If you can write files without delaying the turn, you may also write the same
HTML to this runtime path for diagnostics:
${runtimeOutputPath}`,
    runId,
    runtimeOutputPath,
  };
}

async function callCodexAgent({ artifactDir, prompt, skillPath, timeoutMs }) {
  const workspaceDir = mkdtempSync(path.join(tmpdir(), "skill-ab-agent-"));
  const outputDir = path.join(workspaceDir, "output");
  const finalResponsePath = path.join(workspaceDir, "final-response.md");
  const agentPromptPath = path.join(workspaceDir, "agent-prompt.txt");
  const agentLogPath = path.join(artifactDir, "codex-agent.log");
  mkdirSync(outputDir, { recursive: true });
  writeFileSync(agentPromptPath, prompt, "utf8");
  writeFileSync(path.join(artifactDir, "agent-workspace-path.txt"), `${workspaceDir}\n`, "utf8");

  const args = [
    "exec",
    "--ignore-user-config",
    "--ignore-rules",
    "--skip-git-repo-check",
    "--ephemeral",
    "--sandbox",
    "workspace-write",
    "-C",
    workspaceDir,
    "-c",
    'approval_policy="never"',
    "-c",
    `skills.config=[{path=${tomlString(skillPath)},enabled=true}]`,
    "-o",
    finalResponsePath,
  ];
  if (process.env.SKILL_AB_AGENT_MODEL) {
    args.push("-m", process.env.SKILL_AB_AGENT_MODEL);
  }
  args.push(prompt);

  const result = await runCommand(process.env.SKILL_AB_CODEX_BIN || process.env.CODEX_CLI_PATH || "codex", args, {
    cwd: harnessRoot,
    env: codexAgentEnv(),
    timeoutMs,
  });
  writeFileSync(agentLogPath, `${result.stdout}\n${result.stderr}`.trim(), "utf8");

  const htmlPath = path.join(outputDir, "index.html");
  if (existsSync(htmlPath)) {
    return {
      output: readFileSync(htmlPath, "utf8"),
      prompt,
    };
  }

  const fallback = existsSync(finalResponsePath) ? readFileSync(finalResponsePath, "utf8") : result.stdout;
  return {
    output: fallback,
    prompt,
  };
}

async function callDevfinityAgent({
  artifactDir,
  prompt,
  runtimeOutputPath,
  skillName,
  skillPath,
  skillSourcePath,
  skillText,
  timeoutMs,
  variant,
}) {
  const outputDir = path.join(artifactDir, "devfinity-output");
  mkdirSync(outputDir, { recursive: true });

  const promptPath = path.join(artifactDir, "devfinity-prompt.txt");
  const resultPath = path.join(outputDir, "result.json");
  const logPath = path.join(artifactDir, "devfinity-agent.log");
  const workspaceDir = path.join(outputDir, "workspace");
  const stateDir = resolveReusableDevfinityStateDir();
  const skillBundle = prepareDevfinitySkillBundle({
    artifactDir,
    skillName,
    skillPath,
    skillSourcePath,
    skillText,
  });
  writeFileSync(promptPath, prompt, "utf8");
  writeFileSync(path.join(artifactDir, "devfinity-state-path.txt"), `${stateDir}\n`, "utf8");

  const args = [
    "run",
    "-p",
    "devfinity",
    "--",
    "--state-dir",
    stateDir,
    "agent-run",
    "--skill",
    skillBundle.skillFile,
    "--workspace",
    workspaceDir,
    "--output",
    resultPath,
    "--runtime-output-path",
    runtimeOutputPath,
    "--reply-timeout-ms",
    String(process.env.SKILL_AB_DEVFINITY_REPLY_TIMEOUT_MS || 1200000),
    "--label",
    variant,
    promptPath,
  ];

  let result;
  try {
    result = await withDevfinityAgentRunLock(stateDir, async () => {
      return await runCommand(process.env.SKILL_AB_DEVFINITY_CARGO_BIN || "cargo", args, {
        cwd: repoRoot,
        env: devfinityClientEnv({ variant }),
        timeoutMs,
      });
    });
    writeFileSync(logPath, `${result.stdout}\n${result.stderr}`.trim(), "utf8");
  } catch (error) {
    writeFileSync(logPath, `${error.stdout || ""}\n${error.stderr || ""}\n${error.message}`.trim(), "utf8");
    throw error;
  }

  if (!existsSync(resultPath)) {
    throw new Error(`Devfinity runner did not write ${displayPath(resultPath)}`);
  }
  const payload = JSON.parse(readFileSync(resultPath, "utf8"));
  return {
    output: String(payload.html || payload.finalReply || ""),
    prompt,
  };
}

async function callDisposableDevfinityAgent({
  artifactDir,
  prompt,
  runtimeOutputPath,
  skillName,
  skillPath,
  skillSourcePath,
  skillText,
  timeoutMs,
  upstreamKey,
  variant,
}) {
  const outputDir = path.join(artifactDir, "devfinity-output");
  mkdirSync(outputDir, { recursive: true });

  const promptPath = path.join(artifactDir, "devfinity-prompt.txt");
  const resultPath = path.join(outputDir, "result.json");
  const logPath = path.join(artifactDir, "devfinity-agent.log");
  const stateDir = makeDevfinityStateDir({ artifactDir, variant });
  const skillBundle = prepareDevfinitySkillBundle({
    artifactDir,
    skillName,
    skillPath,
    skillSourcePath,
    skillText,
  });
  writeFileSync(promptPath, prompt, "utf8");

  const env = devfinityEnv({
    promptPath,
    resultPath,
    runtimeOutputPath,
    skillBundleDir: skillBundle.bundleRoot,
    stateDir,
    upstreamKey,
    variant,
  });
  const cleanupEnv = devfinityCleanupEnv(env);
  registerDevfinityStateDir({
    artifactDir,
    cleanupEnv,
    stateDir,
    variant,
  });
  const args = [
    "run",
    "-p",
    "devfinity",
    "--",
    "--state-dir",
    stateDir,
    "up",
    "--headless",
  ];
  if (process.env.SKILL_AB_DEVFINITY_DOCKER_RUNTIME === "1") {
    args.push("--docker-runtime");
  }
  args.push("--", process.execPath, "finite-skills/ab-testing/scripts/run-devfinity-agent-turn.mjs");

  let result;
  try {
    result = await runCommand(process.env.SKILL_AB_DEVFINITY_CARGO_BIN || "cargo", args, {
      cwd: repoRoot,
      env,
      timeoutMs,
    });
    writeFileSync(logPath, `${result.stdout}\n${result.stderr}`.trim(), "utf8");
  } catch (error) {
    writeFileSync(logPath, `${error.stdout || ""}\n${error.stderr || ""}\n${error.message}`.trim(), "utf8");
    throw error;
  } finally {
    cleanupDevfinityStateDirSync(stateDir, {
      cleanupEnv,
      logPath,
      reason: "provider finally",
    });
  }

  if (!existsSync(resultPath)) {
    throw new Error(`Devfinity runner did not write ${displayPath(resultPath)}`);
  }
  const payload = JSON.parse(readFileSync(resultPath, "utf8"));
  return {
    output: String(payload.html || payload.finalReply || ""),
    prompt,
  };
}

function resolveReusableDevfinityStateDir() {
  const configured = process.env.SKILL_AB_DEVFINITY_STATE_DIR || process.env.DEVFINITY_STATE_DIR || ".local-state/devfinity";
  return normalizeDevfinityStateRoot(resolveRepoRelativePath(configured));
}

function normalizeDevfinityStateRoot(candidate) {
  if (existsSync(path.join(candidate, "env")) && path.basename(candidate) === "default" && path.basename(path.dirname(candidate)) === "runs") {
    return path.dirname(path.dirname(candidate));
  }
  return candidate;
}

function makeDevfinityStateDir({ artifactDir, variant }) {
  const stateRoot = path.resolve(process.env.SKILL_AB_DEVFINITY_STATE_ROOT || path.join(tmpdir(), "finite-skills-ab-devfinity"));
  mkdirSync(stateRoot, { recursive: true });
  const stateDir = mkdtempSync(path.join(stateRoot, `${sanitize(variant)}-`));
  writeFileSync(path.join(artifactDir, "devfinity-state-path.txt"), `${stateDir}\n`, "utf8");
  return stateDir;
}

function prepareDevfinitySkillBundle({ artifactDir, skillName, skillPath, skillSourcePath, skillText }) {
  const bundleRoot = path.join(artifactDir, "devfinity-skill-bundle");
  rmSync(bundleRoot, { recursive: true, force: true });

  const sourceDir = path.dirname(skillSourcePath);
  const relDir = skillRelativeDir(skillSourcePath, skillName);
  const targetDir = path.join(bundleRoot, relDir);
  mkdirSync(path.dirname(targetDir), { recursive: true });
  if (existsSync(sourceDir)) {
    cpSync(sourceDir, targetDir, { recursive: true, force: true });
  } else {
    mkdirSync(targetDir, { recursive: true });
  }
  writeFileSync(path.join(targetDir, "SKILL.md"), `${skillText.trim()}\n`, "utf8");
  writeFileSync(
    path.join(artifactDir, "devfinity-skill-bundle.json"),
    JSON.stringify(
      {
        bundleRoot,
        installedSkillDir: relDir,
        sourceSkillPath: skillSourcePath,
        editedSkillPath: skillPath,
      },
      null,
      2,
    ),
    "utf8",
  );
  return { bundleRoot, relDir, skillFile: path.join(targetDir, "SKILL.md") };
}

function skillRelativeDir(skillSourcePath, skillName) {
  const skillsRoot = path.resolve(harnessRoot, "../skills");
  const sourceDir = path.dirname(skillSourcePath);
  const relative = path.relative(skillsRoot, sourceDir);
  if (relative && !relative.startsWith("..") && !path.isAbsolute(relative)) {
    return relative;
  }
  return path.join("ab-test", sanitize(skillName || path.basename(sourceDir)));
}

function devfinityEnv({ promptPath, resultPath, runtimeOutputPath, skillBundleDir, stateDir, upstreamKey, variant }) {
  const env = { ...process.env };
  for (const name of [
    "OPENAI_API_KEY",
    "SKILL_AB_FINITE_PRIVATE_KEY",
    "FINITE_PRIVATE_API_KEY",
    "FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE",
  ]) {
    delete env[name];
  }
  env.FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY = upstreamKey;
  env.SKILL_AB_DEVFINITY_PROMPT_FILE = promptPath;
  env.SKILL_AB_DEVFINITY_RESULT_FILE = resultPath;
  env.SKILL_AB_DEVFINITY_RUNTIME_OUTPUT_PATH = runtimeOutputPath;
  env.SKILL_AB_DEVFINITY_SKILL_BUNDLE_DIR = skillBundleDir;
  env.SKILL_AB_DEVFINITY_VARIANT = variant;
  const runIdentity = `${variant}:${promptPath}:${stateDir}`;
  env.DEVFINITY_PORT_OFFSET = env.SKILL_AB_DEVFINITY_PORT_OFFSET || String(devfinityPortOffset(runIdentity));
  env.DEVFINITY_APPLE_CONTAINER_NAME_PREFIX =
    env.SKILL_AB_DEVFINITY_APPLE_CONTAINER_NAME_PREFIX || `finite-ab-${hashString(runIdentity).slice(0, 10)}`;
  env.DEVFINITY_RUNTIME_IMAGE_REF =
    env.SKILL_AB_DEVFINITY_RUNTIME_IMAGE_REF || `finite-agent-runtime:skill-ab-${hashString(runIdentity).slice(0, 12)}`;
  return env;
}

function devfinityClientEnv({ variant }) {
  const env = { ...process.env };
  for (const name of [
    "OPENAI_API_KEY",
    "SKILL_AB_FINITE_PRIVATE_KEY",
    "FINITE_PRIVATE_API_KEY",
    "FC_RUNNER_FINITE_PRIVATE_API_KEY_OVERRIDE",
  ]) {
    delete env[name];
  }
  env.SKILL_AB_DEVFINITY_VARIANT = variant;
  return env;
}

async function withDevfinityAgentRunLock(stateDir, callback) {
  const lockDir = path.join(stateDir, "runs/default/agent-run.lock");
  const timeoutMs = Number(process.env.SKILL_AB_DEVFINITY_LOCK_TIMEOUT_MS || process.env.SKILL_AB_DEVFINITY_TIMEOUT_MS || 1800000);
  const deadline = Date.now() + timeoutMs;
  await acquireDirectoryLock(lockDir, deadline);
  try {
    return await callback();
  } finally {
    rmSync(lockDir, { recursive: true, force: true });
  }
}

async function acquireDirectoryLock(lockDir, deadline) {
  mkdirSync(path.dirname(lockDir), { recursive: true });
  while (true) {
    try {
      mkdirSync(lockDir);
      writeFileSync(path.join(lockDir, "owner.json"), JSON.stringify({ pid: process.pid, startedAt: new Date().toISOString() }, null, 2), "utf8");
      return;
    } catch (error) {
      if (error.code !== "EEXIST") {
        throw error;
      }
      if (isStaleLock(lockDir)) {
        rmSync(lockDir, { recursive: true, force: true });
        continue;
      }
      if (Date.now() > deadline) {
        throw new Error(`Timed out waiting for Devfinity agent-run lock at ${displayPath(lockDir)}`);
      }
      await delay(500);
    }
  }
}

function isStaleLock(lockDir) {
  try {
    const owner = JSON.parse(readFileSync(path.join(lockDir, "owner.json"), "utf8"));
    const pid = Number(owner.pid);
    if (Number.isInteger(pid) && pid > 0 && !processIsAlive(pid)) {
      return true;
    }
    const startedAt = Date.parse(owner.startedAt);
    return Number.isFinite(startedAt) && Date.now() - startedAt > 2 * 60 * 60 * 1000;
  } catch {
    try {
      return Date.now() - statSync(lockDir).mtimeMs > 30_000;
    } catch {
      return false;
    }
  }
}

function processIsAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function devfinityPortOffset(runIdentity) {
  return 1000 + (Number.parseInt(hashString(runIdentity).slice(0, 6), 16) % 20000);
}

function hashString(value) {
  let hash = 2166136261;
  for (const byte of Buffer.from(String(value))) {
    hash ^= byte;
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

async function runCommand(command, args, { cwd = harnessRoot, env = process.env, timeoutMs }) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd,
      detached: true,
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    let settled = false;
    let killTimeout = null;
    const output = () => ({
      stdout: Buffer.concat(stdout).toString("utf8"),
      stderr: Buffer.concat(stderr).toString("utf8"),
    });
    const timeout =
      timeoutMs > 0
        ? setTimeout(() => {
            if (settled) {
              return;
            }
            settled = true;
            killProcessGroup(child, "SIGTERM");
            killTimeout = setTimeout(() => killProcessGroup(child, "SIGKILL"), 5000);
            const result = output();
            reject(commandError(`${path.basename(command)} timed out after ${timeoutMs}ms`, result));
          }, timeoutMs)
        : null;

    const settle = (callback, value) => {
      if (settled) {
        return;
      }
      settled = true;
      if (timeout) {
        clearTimeout(timeout);
      }
      if (killTimeout) {
        clearTimeout(killTimeout);
      }
      callback(value);
    };

    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stderr.on("data", (chunk) => stderr.push(chunk));
    child.on("error", (error) => {
      settle(reject, error);
    });
    child.on("exit", (code, signal) => {
      const result = output();
      if (code === 0) {
        settle(resolve, result);
        return;
      }
      const tail = `${result.stdout}\n${result.stderr}`.slice(-4000).trim();
      settle(
        reject,
        commandError(`${path.basename(command)} exited with ${signal ?? code}${tail ? `\n${tail}` : ""}`, result),
      );
    });
  });
}

function commandError(message, result) {
  const error = new Error(message);
  error.stdout = result.stdout;
  error.stderr = result.stderr;
  return error;
}

function killProcessGroup(child, signal) {
  if (!child.pid) {
    return;
  }
  try {
    process.kill(-child.pid, signal);
  } catch {
    child.kill(signal);
  }
}

function codexAgentEnv() {
  const passThrough = [
    "HOME",
    "LANG",
    "LC_ALL",
    "LOGNAME",
    "PATH",
    "SHELL",
    "SSH_AUTH_SOCK",
    "TERM",
    "TMP",
    "TMPDIR",
    "USER",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
  ];
  const env = {};
  for (const name of passThrough) {
    if (process.env[name]) {
      env[name] = process.env[name];
    }
  }
  return env;
}

function tomlString(value) {
  return JSON.stringify(String(value));
}

function buildHtmlRepairMessages({ agentOutput, brief, caseTitle }) {
  return [
    {
      role: "system",
      content:
        "You convert an agent's prior response into the exact artifact required by a visual review harness. Return only a complete self-contained HTML document. The first bytes must be <!doctype html>.",
    },
    {
      role: "user",
      content: `The previous response did not contain a complete HTML document.

Original build prompt:
${brief}

Case title: ${caseTitle}

Previous response:
${agentOutput}

Now return only the complete single-file HTML artifact.`,
    },
  ];
}

async function callResponsesApi({ apiKey, baseUrl, messages, model, maxOutputTokens, timeoutMs, providerLabel }) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(`${baseUrl}/responses`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${apiKey}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        input: messages,
        max_output_tokens: maxOutputTokens,
        model,
      }),
      signal: controller.signal,
    });

    const data = await response.json().catch(() => ({}));
    if (!response.ok) {
      const message = data?.error?.message || `${response.status} ${response.statusText}`;
      throw new Error(`${providerLabel} request failed: ${message}`);
    }

    return {
      output: responseOutputText(data, providerLabel),
      tokenUsage: {
        total: data?.usage?.total_tokens,
        prompt: data?.usage?.input_tokens,
        completion: data?.usage?.output_tokens,
      },
    };
  } finally {
    clearTimeout(timeout);
  }
}

function responseOutputText(data, providerLabel) {
  if (typeof data.output_text === "string" && data.output_text.trim()) {
    return data.output_text;
  }
  const parts = [];
  for (const item of data.output ?? []) {
    for (const content of item.content ?? []) {
      if (typeof content.text === "string") {
        parts.push(content.text);
      }
    }
  }
  const output = parts.join("\n").trim();
  if (!output) {
    throw new Error(`${providerLabel} response did not include text output`);
  }
  return output;
}

function extractHtml(output) {
  const fence = output.match(/```(?:html)?\s*([\s\S]*?)```/i);
  const candidate = fence ? fence[1] : output;
  const htmlStart = findHtmlStart(candidate);
  if (htmlStart >= 0) {
    const sliced = candidate.slice(htmlStart);
    const end = sliced.toLowerCase().lastIndexOf("</html>");
    return {
      found: true,
      html: end >= 0 ? sliced.slice(0, end + "</html>".length).trim() : sliced.trim(),
    };
  }

  return {
    found: false,
    html: null,
  };
}

function truncateForError(value, limit = 600) {
  const text = String(value ?? "").trim().replace(/\s+/g, " ");
  if (text.length <= limit) {
    return text;
  }
  return `${text.slice(0, limit)}...`;
}

function findHtmlStart(text) {
  const lower = text.toLowerCase();
  const doctype = lower.indexOf("<!doctype");
  const html = lower.indexOf("<html");
  if (doctype >= 0 && html >= 0) {
    return Math.min(doctype, html);
  }
  return Math.max(doctype, html);
}

function parseSkillName(skillText) {
  const match = skillText.match(/^name:\s*"?([^"\n]+)"?\s*$/m);
  return match?.[1]?.trim();
}

function sanitize(value) {
  return String(value)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 80) || "item";
}

function shouldRepairHtml(config) {
  if (process.env.SKILL_AB_REPAIR_HTML === "0") {
    return false;
  }
  return config.repairHtml !== false;
}

function addTokenUsage(left = {}, right = {}) {
  return {
    total: addNumbers(left.total, right.total),
    prompt: addNumbers(left.prompt, right.prompt),
    completion: addNumbers(left.completion, right.completion),
  };
}

function addNumbers(left, right) {
  if (typeof left !== "number" && typeof right !== "number") {
    return undefined;
  }
  return (Number(left) || 0) + (Number(right) || 0);
}
