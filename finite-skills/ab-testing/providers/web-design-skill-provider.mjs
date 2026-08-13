import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const providerDir = path.dirname(fileURLToPath(import.meta.url));
const harnessRoot = path.resolve(providerDir, "..");
const repoRoot = path.resolve(harnessRoot, "../..");
const finitePrivateDefaultBaseUrl = "https://kimi-k2-6.finite.containers.tinfoil.dev/v1";
const finitePrivateDefaultModel = "glm-5-2";
const openAIDefaultBaseUrl = "https://api.openai.com/v1";
const openAIDefaultModel = "gpt-5-mini";

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
    const skillText = readFileSync(skillPath, "utf8");
    const skillName = parseSkillName(skillText) ?? path.basename(path.dirname(skillPath));
    const artifactDir = path.resolve(harnessRoot, this.config.outputDir ?? "./runs/latest/artifacts", caseId, sanitize(this.variant));
    mkdirSync(artifactDir, { recursive: true });

    const agentPrompt = buildAgentPrompt({
      brief: prompt,
      caseTitle: vars.title ?? vars.caseId ?? caseId,
      skillName,
      skillPath,
      skillText,
    });

    const startedAt = new Date().toISOString();
    let output;
    let modelProvider;
    let model;
    let tokenUsage;

    if (isMockMode(this.config)) {
      model = "mock";
      modelProvider = { name: "mock", baseUrl: null, keySource: null };
      output = mockHtml({ caseId, caseTitle: vars.title ?? caseId, skillName, variant: this.variant });
      tokenUsage = { total: 0, prompt: 0, completion: 0 };
    } else {
      modelProvider = resolveModelProvider(this.config);
      model = modelProvider.model;
      const response = await callResponsesApi({
        apiKey: modelProvider.apiKey,
        baseUrl: modelProvider.baseUrl,
        messages: agentPrompt.messages,
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
    if (!isMockMode(this.config) && !extraction.found && shouldRepairHtml(this.config)) {
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

    const html = extraction.html;
    writeFileSync(path.join(artifactDir, "index.html"), html, "utf8");
    writeFileSync(path.join(artifactDir, "output.raw.md"), output, "utf8");
    writeFileSync(path.join(artifactDir, "prompt.txt"), agentPrompt.transcript, "utf8");
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
      prompt: agentPrompt.transcript,
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
      "A non-mock skill A/B run needs a model API key.",
      "Run `just dev inference-key` to cache a Finite Private key locally, set `FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY`,",
      "or use `SKILL_AB_PROVIDER=openai OPENAI_API_KEY=... pnpm run ab`.",
      "Use `pnpm run ab:mock` to test the harness without API calls.",
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
      "OPENAI_API_KEY is required when SKILL_AB_PROVIDER=openai. Use `pnpm run ab:mock` to test the harness without API calls.",
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
  ].filter(Boolean);
  for (const keyFile of keyFileCandidates) {
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
    html: `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Non-HTML Output</title>
  <style>
    body { margin: 0; font-family: ui-sans-serif, system-ui, sans-serif; background: #f6f4ef; color: #181818; }
    main { max-width: 900px; margin: 48px auto; padding: 0 24px; }
    pre { white-space: pre-wrap; background: white; border: 1px solid #d8d2c5; padding: 20px; }
  </style>
</head>
<body>
  <main>
    <h1>Provider returned non-HTML output</h1>
    <pre>${escapeHtml(output)}</pre>
  </main>
</body>
</html>`,
  };
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

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function isMockMode(config) {
  return process.env.SKILL_AB_MOCK === "1" || config.mock === true;
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

function mockHtml({ caseId, caseTitle, skillName, variant }) {
  const accent = variant === "skill-a" ? "#176f6b" : "#b2452f";
  const secondary = variant === "skill-a" ? "#d9efe8" : "#f3ddd3";
  const panel = variant === "skill-a" ? "#f7fbf8" : "#fff8f4";
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escapeHtml(caseTitle)}</title>
  <style>
    * { box-sizing: border-box; }
    body { margin: 0; font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; color: #20201d; background: ${panel}; }
    main { min-height: 100vh; display: grid; grid-template-columns: 280px 1fr; }
    aside { border-right: 1px solid #d8d1c4; padding: 28px; background: #fffdf8; }
    .brand { font-weight: 800; letter-spacing: 0; font-size: 18px; margin-bottom: 28px; }
    nav { display: grid; gap: 8px; }
    nav span { display: block; padding: 10px 12px; border-radius: 6px; color: #5e5a52; }
    nav span:first-child { color: #161613; background: ${secondary}; }
    section { padding: 34px; }
    .top { display: flex; justify-content: space-between; gap: 20px; align-items: start; margin-bottom: 28px; }
    h1 { margin: 0; font-size: clamp(28px, 4vw, 58px); line-height: .95; max-width: 720px; }
    .tag { padding: 8px 10px; border: 1px solid #d8d1c4; border-radius: 999px; background: #fffdf8; font-size: 13px; white-space: nowrap; }
    .grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; }
    .panel { background: #fffdf8; border: 1px solid #d8d1c4; border-radius: 8px; padding: 18px; min-height: 160px; }
    .panel strong { display: block; font-size: 13px; color: #69645a; margin-bottom: 16px; }
    .metric { font-size: 42px; font-weight: 800; color: ${accent}; }
    .bar { height: 10px; background: #e7dfd2; border-radius: 99px; overflow: hidden; margin-top: 18px; }
    .bar span { display: block; height: 100%; width: 68%; background: ${accent}; }
    .wide { grid-column: span 2; }
    .rows { display: grid; gap: 10px; }
    .row { display: flex; justify-content: space-between; border-top: 1px solid #ece5d8; padding-top: 10px; font-size: 14px; }
    @media (max-width: 820px) {
      main { grid-template-columns: 1fr; }
      aside { border-right: 0; border-bottom: 1px solid #d8d1c4; }
      .grid { grid-template-columns: 1fr; }
      .wide { grid-column: span 1; }
      .top { display: grid; }
    }
  </style>
</head>
<body>
  <main data-case="${escapeHtml(caseId)}" data-skill="${escapeHtml(skillName)}">
    <aside>
      <div class="brand">Local Design Review</div>
      <nav><span>Overview</span><span>Signals</span><span>Work queue</span><span>Settings</span></nav>
    </aside>
    <section>
      <div class="top">
        <h1>${escapeHtml(caseTitle)}</h1>
        <div class="tag">${escapeHtml(variant)} mock artifact</div>
      </div>
      <div class="grid">
        <div class="panel"><strong>Primary signal</strong><div class="metric">68%</div><div class="bar"><span></span></div></div>
        <div class="panel"><strong>Open work</strong><div class="metric">14</div><div class="bar"><span style="width:44%"></span></div></div>
        <div class="panel"><strong>Confidence</strong><div class="metric">A-</div><div class="bar"><span style="width:82%"></span></div></div>
        <div class="panel wide"><strong>Recent activity</strong><div class="rows"><div class="row"><span>North queue stabilized</span><b>2m</b></div><div class="row"><span>Forecast risk moved down</span><b>9m</b></div><div class="row"><span>Review packet generated</span><b>18m</b></div></div></div>
        <div class="panel"><strong>Next action</strong><p>Review the strongest visual hierarchy, spacing, and first-viewport clarity. This is mock output for harness verification.</p></div>
      </div>
    </section>
  </main>
</body>
</html>`;
}
