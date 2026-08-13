import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { createReadStream, existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const latestDir = path.join(harnessRoot, "runs/latest");
const editableDir = path.join(harnessRoot, "runs/editable");
const editableStatePath = path.join(editableDir, "state.json");
const host = process.env.SKILL_AB_HOST || "127.0.0.1";
const port = Number(process.env.SKILL_AB_PORT || process.env.PORT || 8787);
const maxBodyBytes = 4 * 1024 * 1024;

const variants = [
  {
    label: "website-building-finite",
    sourcePath: path.resolve(harnessRoot, "../skills/software-development/website-building-finite/SKILL.md"),
    variant: "skill-a",
  },
  {
    label: "impeccable-finite",
    sourcePath: path.resolve(harnessRoot, "../skills/software-development/impeccable-finite/SKILL.md"),
    variant: "skill-b",
  },
];

let currentJob = null;

mkdirSync(editableDir, { recursive: true });

const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url, `http://${request.headers.host || `${host}:${port}`}`);
    if (url.pathname === "/api/state" && request.method === "GET") {
      return sendJson(response, 200, readEditorState());
    }
    if (url.pathname === "/api/regenerate" && request.method === "POST") {
      return handleRegenerate(request, response);
    }
    if (url.pathname.startsWith("/api/jobs/") && request.method === "GET") {
      const jobId = url.pathname.split("/").pop();
      if (!currentJob || currentJob.id !== jobId) {
        return sendJson(response, 404, { error: "Job not found" });
      }
      return sendJson(response, 200, publicJob(currentJob));
    }
    if (url.pathname.startsWith("/api/")) {
      return sendJson(response, 404, { error: "Not found" });
    }
    return serveStatic(url.pathname, response);
  } catch (error) {
    return sendJson(response, 500, { error: error.message });
  }
});

server.listen(port, host, () => {
  console.log(`Finite Skills A/B review server: http://${host}:${port}/review/index.html`);
});

async function handleRegenerate(request, response) {
  if (currentJob?.status === "running") {
    return sendJson(response, 409, { error: "A regeneration job is already running", job: publicJob(currentJob) });
  }

  const payload = JSON.parse(await readBody(request));
  const prompt = String(payload.prompt || "").trim();
  if (!prompt) {
    return sendJson(response, 400, { error: "Prompt is required" });
  }

  const skillTexts = payload.skills || {};
  for (const variant of variants) {
    const text = String(skillTexts[variant.variant] || "").trim();
    if (!text) {
      return sendJson(response, 400, { error: `${variant.variant} skill text is required` });
    }
  }

  const title = String(payload.title || titleFromBrief(prompt)).trim();
  const maxConcurrency = normalizeMaxConcurrency(payload.maxConcurrency);
  const mock = Boolean(payload.mock);

  for (const variant of variants) {
    const file = editableSkillPath(variant.variant);
    mkdirSync(path.dirname(file), { recursive: true });
    writeFileSync(file, `${String(skillTexts[variant.variant]).trim()}\n`, "utf8");
  }
  writeFileSync(
    editableStatePath,
    JSON.stringify(
      {
        caseId: sanitize(title),
        maxConcurrency,
        mock,
        prompt,
        title,
        updatedAt: new Date().toISOString(),
      },
      null,
      2,
    ),
    "utf8",
  );

  currentJob = startRegenerateJob({ maxConcurrency, mock, prompt, title });
  return sendJson(response, 202, { job: publicJob(currentJob) });
}

function startRegenerateJob({ maxConcurrency, mock, prompt, title }) {
  const job = {
    exitCode: null,
    finishedAt: null,
    id: randomUUID(),
    logs: [],
    startedAt: new Date().toISOString(),
    status: "running",
  };
  const env = {
    ...process.env,
    SKILL_AB_CASE_TITLE: title,
    SKILL_AB_MAX_CONCURRENCY: String(maxConcurrency),
    SKILL_AB_SKILL_A_PATH: editableSkillPath("skill-a"),
    SKILL_AB_SKILL_B_PATH: editableSkillPath("skill-b"),
  };
  if (mock) {
    env.SKILL_AB_MOCK = "1";
  } else {
    delete env.SKILL_AB_MOCK;
  }

  const child = spawn(process.execPath, ["scripts/run-prompt.mjs", prompt], {
    cwd: harnessRoot,
    env,
    stdio: ["ignore", "pipe", "pipe"],
  });

  child.stdout.on("data", (chunk) => appendLog(job, chunk));
  child.stderr.on("data", (chunk) => appendLog(job, chunk));
  child.on("error", (error) => {
    appendLog(job, `${error.message}\n`);
    job.status = "failed";
    job.finishedAt = new Date().toISOString();
  });
  child.on("exit", (code, signal) => {
    job.exitCode = signal ?? code;
    job.status = code === 0 ? "complete" : "failed";
    job.finishedAt = new Date().toISOString();
  });

  return job;
}

function appendLog(job, chunk) {
  job.logs.push(String(chunk));
  if (job.logs.length > 240) {
    job.logs.splice(0, job.logs.length - 240);
  }
}

function publicJob(job) {
  return {
    exitCode: job.exitCode,
    finishedAt: job.finishedAt,
    id: job.id,
    logs: job.logs.join(""),
    startedAt: job.startedAt,
    status: job.status,
  };
}

function readEditorState() {
  const editableState = readJson(editableStatePath, {});
  const manifest = readJson(path.join(latestDir, "review/manifest.json"), {});
  const firstArtifact = manifest.artifacts?.[0];
  const prompt = editableState.prompt || firstArtifact?.metadata?.brief || defaultPrompt();
  const title = editableState.title || firstArtifact?.caseTitle || titleFromBrief(prompt);

  return {
    maxConcurrency: editableState.maxConcurrency || normalizeMaxConcurrency(process.env.SKILL_AB_MAX_CONCURRENCY || 1),
    mock: typeof editableState.mock === "boolean" ? editableState.mock : process.env.SKILL_AB_MOCK === "1",
    prompt,
    title,
    variants: variants.map((variant) => {
      const editablePath = editableSkillPath(variant.variant);
      const skillPath = existsSync(editablePath) ? editablePath : variant.sourcePath;
      return {
        label: variant.label,
        skillText: readFileSync(skillPath, "utf8"),
        sourcePath: displayPath(variant.sourcePath),
        variant: variant.variant,
      };
    }),
  };
}

function serveStatic(pathname, response) {
  const requestPath = pathname === "/" ? "/review/index.html" : decodeURIComponent(pathname);
  const filePath = path.resolve(latestDir, `.${requestPath}`);
  if (!filePath.startsWith(`${latestDir}${path.sep}`) && filePath !== latestDir) {
    response.writeHead(403);
    response.end("Forbidden");
    return;
  }
  if (!existsSync(filePath)) {
    response.writeHead(404);
    response.end("Not found");
    return;
  }
  const stat = statSync(filePath);
  if (stat.isDirectory()) {
    response.writeHead(302, { Location: path.posix.join(requestPath, "index.html") });
    response.end();
    return;
  }
  response.writeHead(200, { "Content-Type": contentType(filePath) });
  createReadStream(filePath).pipe(response);
}

function contentType(filePath) {
  switch (path.extname(filePath).toLowerCase()) {
    case ".html":
      return "text/html; charset=utf-8";
    case ".json":
      return "application/json; charset=utf-8";
    case ".js":
      return "text/javascript; charset=utf-8";
    case ".css":
      return "text/css; charset=utf-8";
    case ".png":
      return "image/png";
    case ".md":
    case ".txt":
      return "text/plain; charset=utf-8";
    default:
      return "application/octet-stream";
  }
}

async function readBody(request) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > maxBodyBytes) {
      throw new Error("Request body is too large");
    }
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString("utf8");
}

function sendJson(response, statusCode, data) {
  response.writeHead(statusCode, { "Content-Type": "application/json; charset=utf-8" });
  response.end(JSON.stringify(data));
}

function readJson(file, fallback) {
  if (!existsSync(file)) {
    return fallback;
  }
  return JSON.parse(readFileSync(file, "utf8"));
}

function editableSkillPath(variant) {
  return path.join(editableDir, "skills", variant, "SKILL.md");
}

function titleFromBrief(value) {
  const firstLine = value.split(/\r?\n/).find((line) => line.trim())?.trim() || "Custom prompt";
  return firstLine.replace(/\s+/g, " ").slice(0, 72);
}

function sanitize(value) {
  return String(value)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 80) || "custom-prompt";
}

function normalizeMaxConcurrency(value) {
  const number = Number(value);
  if (!Number.isFinite(number)) {
    return 1;
  }
  return Math.max(1, Math.min(8, Math.floor(number)));
}

function displayPath(filePath) {
  const relative = path.relative(harnessRoot, filePath);
  if (relative && !relative.startsWith("..") && !path.isAbsolute(relative)) {
    return relative;
  }
  return filePath;
}

function defaultPrompt() {
  return "Build a first screen for a browser dashboard that helps a renewable-energy operations team scan turbine health, open incidents, weather risk, and dispatch status.";
}
