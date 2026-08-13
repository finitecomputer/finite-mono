import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const latestDir = path.join(harnessRoot, "runs/latest");
const artifactsDir = path.join(latestDir, "artifacts");
const reviewDir = path.join(latestDir, "review");

mkdirSync(reviewDir, { recursive: true });

const artifacts = collectArtifacts();
const screenshots = readJson(path.join(reviewDir, "screenshots.json"), []);
const screenshotByArtifact = new Map();
const artifactKeys = new Set(artifacts.map((artifact) => `${artifact.caseId}/${artifact.variant}`));
for (const screenshot of screenshots) {
  const key = `${screenshot.caseId}/${screenshot.variant}`;
  if (!artifactKeys.has(key)) {
    continue;
  }
  const existing = screenshotByArtifact.get(key) ?? [];
  existing.push(screenshot);
  screenshotByArtifact.set(key, existing);
}

const grouped = groupByCase(artifacts);
writeFileSync(path.join(reviewDir, "manifest.json"), JSON.stringify({ artifacts, screenshots }, null, 2), "utf8");
writeFileSync(path.join(reviewDir, "index.html"), renderReview(grouped, screenshotByArtifact), "utf8");

console.log(`Wrote ${path.join(reviewDir, "index.html")}`);

function collectArtifacts() {
  if (!existsSync(artifactsDir)) {
    return [];
  }
  const items = [];
  for (const caseId of sortedDirs(artifactsDir)) {
    for (const variant of sortedDirs(path.join(artifactsDir, caseId))) {
      const dir = path.join(artifactsDir, caseId, variant);
      const htmlPath = path.join(dir, "index.html");
      const metadataPath = path.join(dir, "metadata.json");
      if (!existsSync(htmlPath) || !existsSync(metadataPath)) {
        continue;
      }
      const metadata = readJson(metadataPath, {});
      if (!isReviewableArtifact(metadata)) {
        continue;
      }
      items.push({
        caseId,
        caseTitle: metadata.caseTitle ?? caseId,
        htmlPath,
        metadata,
        promptPath: path.join(dir, "prompt.txt"),
        rawPath: path.join(dir, "output.raw.md"),
        variant,
      });
    }
  }
  return items;
}

function isReviewableArtifact(metadata) {
  return metadata.runner !== "mock" && metadata.modelProvider !== "mock" && metadata.model !== "mock";
}

function groupByCase(artifacts) {
  const cases = new Map();
  for (const artifact of artifacts) {
    const existing = cases.get(artifact.caseId) ?? {
      brief: artifact.metadata.brief,
      caseId: artifact.caseId,
      caseTitle: artifact.caseTitle,
      variants: [],
    };
    existing.brief ??= artifact.metadata.brief;
    existing.variants.push(artifact);
    cases.set(artifact.caseId, existing);
  }
  return [...cases.values()];
}

function sortedDirs(dir) {
  return readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();
}

function readJson(file, fallback) {
  if (!existsSync(file)) {
    return fallback;
  }
  return JSON.parse(readFileSync(file, "utf8"));
}

function renderReview(cases, screenshotByArtifact) {
  const generatedAt = new Date().toLocaleString();
  const caseCount = cases.length;
  const variantCount = cases.reduce((sum, item) => sum + item.variants.length, 0);
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Finite Skills A/B Review</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f4f1ea;
      --paper: #fffdf8;
      --ink: #1d1d1a;
      --muted: #6f6a60;
      --line: #d8d1c4;
      --accent: #1f6b63;
      --accent-soft: #d9eee8;
      --danger: #9d3e2f;
    }
    * { box-sizing: border-box; }
    body { margin: 0; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background: var(--bg); color: var(--ink); }
    header { position: sticky; top: 0; z-index: 1; background: color-mix(in srgb, var(--paper) 92%, transparent); border-bottom: 1px solid var(--line); backdrop-filter: blur(12px); }
    .header-inner { max-width: 1480px; margin: 0 auto; padding: 18px 24px; display: flex; justify-content: space-between; gap: 16px; align-items: center; }
    h1 { margin: 0; font-size: 22px; line-height: 1.1; }
    .meta { color: var(--muted); font-size: 13px; }
    main { max-width: 1480px; margin: 0 auto; padding: 24px; }
    .editor { background: var(--paper); border: 1px solid var(--line); border-radius: 8px; margin-bottom: 24px; overflow: hidden; }
    .editor-head { padding: 14px 16px; border-bottom: 1px solid var(--line); display: flex; align-items: center; justify-content: space-between; gap: 12px; }
    .editor-title { font-weight: 760; }
    .editor-body { display: grid; gap: 14px; padding: 16px; }
    .field { display: grid; gap: 7px; }
    .field label { color: var(--muted); font-size: 12px; font-weight: 700; text-transform: uppercase; }
    .field textarea, .field input, .field select { width: 100%; border: 1px solid var(--line); border-radius: 6px; padding: 10px; font: 13px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; background: #fff; color: var(--ink); }
    .field textarea { min-height: 112px; resize: vertical; }
    .field.skill textarea { min-height: 260px; }
    .skill-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px; }
    .skill-picker { display: grid; grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr) auto; gap: 8px; align-items: center; }
    .secondary { border: 1px solid var(--line); border-radius: 6px; padding: 9px 12px; background: #fff; font-weight: 700; cursor: pointer; }
    .editor-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; }
    .editor-actions label { display: inline-flex; align-items: center; gap: 7px; color: var(--muted); font-size: 13px; }
    .editor-actions input[type="number"] { width: 74px; }
    .primary { border: 1px solid var(--accent); border-radius: 6px; padding: 9px 12px; color: white; background: var(--accent); font-weight: 750; cursor: pointer; }
    .primary:disabled { opacity: .55; cursor: wait; }
    .status-line { color: var(--muted); font-size: 13px; }
    .job-log { max-height: 220px; overflow: auto; margin: 0; border-top: 1px solid var(--line); padding: 12px 16px; background: #191815; color: #eee9dd; font: 12px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; white-space: pre-wrap; }
    .case { border-top: 1px solid var(--line); padding: 26px 0 34px; }
    .case:first-child { border-top: 0; }
    .case-head { display: flex; align-items: end; justify-content: space-between; gap: 16px; margin-bottom: 14px; }
    h2 { margin: 0; font-size: 18px; }
    .brief { max-width: 880px; margin: 7px 0 0; color: var(--muted); font-size: 13px; line-height: 1.45; }
    .winner { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; color: var(--muted); font-size: 13px; }
    .winner label { display: inline-flex; gap: 6px; align-items: center; padding: 7px 9px; background: var(--paper); border: 1px solid var(--line); border-radius: 6px; color: var(--ink); }
    .variants { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 16px; }
    .variant { background: var(--paper); border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
    .variant-head { padding: 14px 14px 12px; border-bottom: 1px solid var(--line); display: grid; gap: 6px; }
    .variant-title { display: flex; justify-content: space-between; gap: 10px; align-items: center; font-weight: 760; }
    .status { border: 1px solid var(--line); border-radius: 999px; padding: 4px 8px; font-size: 12px; font-weight: 650; background: #fff; color: var(--muted); }
    .status.html { color: var(--accent); background: var(--accent-soft); }
    .status.non-html { color: var(--danger); }
    .skill { color: var(--muted); font-size: 13px; word-break: break-word; }
    .links { display: flex; flex-wrap: wrap; gap: 8px; }
    a, button { color: var(--accent); }
    .links a, .export { border: 1px solid var(--line); border-radius: 6px; padding: 7px 9px; text-decoration: none; font-size: 13px; background: #fff; }
    .shots { display: grid; gap: 12px; padding: 14px; }
    figure { margin: 0; }
    figcaption { color: var(--muted); font-size: 12px; margin-bottom: 6px; }
    img { display: block; width: 100%; border: 1px solid var(--line); border-radius: 6px; background: white; }
    .notes { width: 100%; min-height: 74px; margin-top: 12px; border: 1px solid var(--line); border-radius: 6px; padding: 10px; font: inherit; background: #fff; resize: vertical; }
    .empty { padding: 32px; background: var(--paper); border: 1px solid var(--line); border-radius: 8px; color: var(--muted); }
    .console-errors { margin: 10px 14px 14px; color: var(--danger); font-size: 12px; }
    @media (max-width: 980px) {
      .variants { grid-template-columns: 1fr; }
      .skill-grid { grid-template-columns: 1fr; }
      .skill-picker { grid-template-columns: 1fr; }
      .case-head, .header-inner { align-items: start; flex-direction: column; }
    }
  </style>
</head>
<body>
  <header>
    <div class="header-inner">
      <div>
        <h1>Finite Skills A/B Review</h1>
        <div class="meta">${caseCount} cases · ${variantCount} artifacts · generated ${escapeHtml(generatedAt)}</div>
      </div>
      <button class="export" type="button" id="export-notes">Export notes JSON</button>
    </div>
  </header>
  <main>
    ${renderEditor()}
    <section id="results">
      ${
        cases.length
          ? cases.map((item) => renderCase(item, screenshotByArtifact)).join("\n")
          : '<div class="empty">No real agent artifacts found. Click Generate to run the selected skills through the local Finite agent.</div>'
      }
    </section>
  </main>
  <script>
    const key = "finite-skills-ab-review-notes";
    const state = JSON.parse(localStorage.getItem(key) || "{}");
    for (const input of document.querySelectorAll("[data-save]")) {
      const field = input.getAttribute("data-save");
      if (input.type === "radio") {
        input.checked = state[field] === input.value;
      } else {
        input.value = state[field] || "";
      }
      input.addEventListener("input", () => {
        if (input.type === "radio") {
          state[field] = input.value;
        } else {
          state[field] = input.value;
        }
        localStorage.setItem(key, JSON.stringify(state, null, 2));
      });
    }
    document.getElementById("export-notes").addEventListener("click", () => {
      const blob = new Blob([JSON.stringify(state, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "finite-skills-ab-review-notes.json";
      a.click();
      URL.revokeObjectURL(url);
    });

    const editor = document.getElementById("input-editor");
    const promptInput = document.getElementById("prompt-input");
    const maxConcurrencyInput = document.getElementById("max-concurrency-input");
    const regenerateButton = document.getElementById("regenerate-button");
    const editorStatus = document.getElementById("editor-status");
    const jobLog = document.getElementById("job-log");
    const results = document.getElementById("results");
    let skillCatalog = [];

    loadEditorState();

    regenerateButton.addEventListener("click", async () => {
      const skills = {};
      for (const input of document.querySelectorAll("[data-skill-input]")) {
        const variant = input.getAttribute("data-skill-input");
        const pathInput = document.querySelector("[data-skill-path='" + variant + "']");
        const select = document.querySelector("[data-skill-select='" + variant + "']");
        const selectedOption = select?.selectedOptions?.[0];
        skills[variant] = {
          label: selectedOption?.textContent || variant,
          path: pathInput?.value || "",
          text: input.value,
        };
      }
      setEditorEnabled(false);
      setEditorStatus("Starting real agent generation...");
      jobLog.hidden = false;
      jobLog.textContent = "";
      if (results) {
        results.innerHTML = '<div class="empty">Generating real previews with the local Finite agent. This can take several minutes.</div>';
      }
      try {
        const response = await fetch("/api/regenerate", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            maxConcurrency: Number(maxConcurrencyInput.value || 2),
            prompt: promptInput.value,
            skills,
          }),
        });
        const payload = await response.json();
        if (!response.ok) {
          throw new Error(payload.error || "Regeneration failed to start");
        }
        pollJob(payload.job.id);
      } catch (error) {
        setEditorStatus(error.message);
        setEditorEnabled(true);
      }
    });

    async function loadEditorState() {
      try {
        const response = await fetch("/api/state");
        if (!response.ok) {
          throw new Error("API unavailable");
        }
        const data = await response.json();
        skillCatalog = data.availableSkills || [];
        promptInput.value = data.prompt || "";
        maxConcurrencyInput.value = data.maxConcurrency || 2;
        for (const variant of data.variants || []) {
          const input = document.getElementById("skill-input-" + variant.variant);
          const source = document.getElementById("skill-source-" + variant.variant);
          const pathInput = document.getElementById("skill-path-" + variant.variant);
          populateSkillSelect(variant.variant, variant.selectedSkillPath || variant.sourcePath || "");
          if (input) input.value = variant.skillText || "";
          if (pathInput) pathInput.value = variant.selectedSkillPath || variant.sourcePath || "";
          if (source) source.textContent = variant.sourcePath || "";
        }
        wireSkillPickers();
        setEditorStatus("Ready");
      } catch {
        setEditorStatus("Start the local review server to edit and regenerate.");
        setEditorEnabled(false);
      }
    }

    async function pollJob(jobId) {
      try {
        const response = await fetch("/api/jobs/" + encodeURIComponent(jobId));
        const job = await response.json();
        if (!response.ok) {
          throw new Error(job.error || "Could not read job status");
        }
        jobLog.hidden = false;
        jobLog.textContent = job.logs || "";
        jobLog.scrollTop = jobLog.scrollHeight;
        if (job.status === "complete") {
          setEditorStatus("Generated. Reloading previews...");
          window.location.href = "/review/index.html?t=" + Date.now();
          return;
        }
        if (job.status === "failed") {
          setEditorStatus("Regeneration failed");
          setEditorEnabled(true);
          return;
        }
        setEditorStatus("Generating real previews...");
        setTimeout(() => pollJob(jobId), 1200);
      } catch (error) {
        setEditorStatus(error.message);
        setEditorEnabled(true);
      }
    }

    function setEditorEnabled(enabled) {
      for (const control of editor.querySelectorAll("textarea, input, select, button")) {
        control.disabled = !enabled;
      }
    }

    function setEditorStatus(value) {
      editorStatus.textContent = value;
    }

    function populateSkillSelect(variant, selectedPath) {
      const select = document.getElementById("skill-select-" + variant);
      if (!select) return;
      select.replaceChildren();
      const custom = document.createElement("option");
      custom.value = "";
      custom.textContent = "Custom path";
      select.appendChild(custom);
      for (const skill of skillCatalog) {
        const option = document.createElement("option");
        option.value = skill.path;
        option.textContent = skill.label;
        select.appendChild(option);
      }
      select.value = skillCatalog.some((skill) => skill.path === selectedPath) ? selectedPath : "";
    }

    function wireSkillPickers() {
      for (const select of document.querySelectorAll("[data-skill-select]")) {
        select.addEventListener("change", () => {
          const variant = select.getAttribute("data-skill-select");
          const pathInput = document.querySelector("[data-skill-path='" + variant + "']");
          if (pathInput && select.value) {
            pathInput.value = select.value;
            loadSkillFile(variant, select.value);
          }
        });
      }
      for (const button of document.querySelectorAll("[data-skill-load]")) {
        button.addEventListener("click", () => {
          const variant = button.getAttribute("data-skill-load");
          const pathInput = document.querySelector("[data-skill-path='" + variant + "']");
          loadSkillFile(variant, pathInput?.value || "");
        });
      }
    }

    async function loadSkillFile(variant, skillPath) {
      if (!skillPath) {
        setEditorStatus("Choose a skill file path first.");
        return;
      }
      try {
        setEditorStatus("Loading " + variant + " skill...");
        const response = await fetch("/api/skill?path=" + encodeURIComponent(skillPath));
        const data = await response.json();
        if (!response.ok) {
          throw new Error(data.error || "Could not load skill file");
        }
        const input = document.getElementById("skill-input-" + variant);
        const source = document.getElementById("skill-source-" + variant);
        const pathInput = document.getElementById("skill-path-" + variant);
        if (input) input.value = data.skillText || "";
        if (source) source.textContent = data.path || skillPath;
        if (pathInput) pathInput.value = data.path || skillPath;
        const select = document.getElementById("skill-select-" + variant);
        if (select && skillCatalog.some((skill) => skill.path === data.path)) {
          select.value = data.path;
        }
        setEditorStatus("Loaded " + variant + " skill");
      } catch (error) {
        setEditorStatus(error.message);
      }
    }
  </script>
</body>
</html>`;
}

function renderEditor() {
  return `<section class="editor" id="input-editor">
    <div class="editor-head">
      <div>
        <div class="editor-title">Run Inputs</div>
        <div class="meta">Edit the build prompt and the skill text used for the next generation.</div>
      </div>
      <span class="status-line" id="editor-status">Loading...</span>
    </div>
    <div class="editor-body">
      <div class="field">
        <label for="prompt-input">Build Prompt</label>
        <textarea id="prompt-input" spellcheck="false"></textarea>
      </div>
      <div class="skill-grid">
        <div class="field skill">
          <label for="skill-input-skill-a">skill-a</label>
          <div class="meta" id="skill-source-skill-a"></div>
          <div class="skill-picker">
            <select id="skill-select-skill-a" data-skill-select="skill-a"></select>
            <input id="skill-path-skill-a" data-skill-path="skill-a" type="text" placeholder="Path to SKILL.md">
            <button class="secondary" type="button" data-skill-load="skill-a">Load</button>
          </div>
          <textarea id="skill-input-skill-a" data-skill-input="skill-a" spellcheck="false"></textarea>
        </div>
        <div class="field skill">
          <label for="skill-input-skill-b">skill-b</label>
          <div class="meta" id="skill-source-skill-b"></div>
          <div class="skill-picker">
            <select id="skill-select-skill-b" data-skill-select="skill-b"></select>
            <input id="skill-path-skill-b" data-skill-path="skill-b" type="text" placeholder="Path to SKILL.md">
            <button class="secondary" type="button" data-skill-load="skill-b">Load</button>
          </div>
          <textarea id="skill-input-skill-b" data-skill-input="skill-b" spellcheck="false"></textarea>
        </div>
      </div>
      <div class="editor-actions">
        <button class="primary" type="button" id="regenerate-button">Generate</button>
        <span class="status-line">Devfinity local Finite agent</span>
        <label>Concurrency <input id="max-concurrency-input" type="number" min="1" max="8" step="1"></label>
      </div>
    </div>
    <pre class="job-log" id="job-log" hidden></pre>
  </section>`;
}

function renderCase(item, screenshotByArtifact) {
  return `<section class="case">
    <div class="case-head">
      <div>
        <h2>${escapeHtml(item.caseTitle)}</h2>
        <div class="meta">${escapeHtml(item.caseId)}</div>
        ${item.brief ? `<p class="brief">${escapeHtml(item.brief)}</p>` : ""}
      </div>
      <div class="winner">
        <span>Winner</span>
        ${item.variants
          .map(
            (variant) =>
              `<label><input type="radio" name="winner-${escapeHtml(item.caseId)}" value="${escapeHtml(variant.variant)}" data-save="winner.${escapeHtml(item.caseId)}"> ${escapeHtml(variant.variant)}</label>`,
          )
          .join("")}
      </div>
    </div>
    <div class="variants">${item.variants.map((variant) => renderVariant(variant, screenshotByArtifact)).join("\n")}</div>
    <textarea class="notes" data-save="notes.${escapeHtml(item.caseId)}" placeholder="Human notes for ${escapeHtml(item.caseTitle)}"></textarea>
  </section>`;
}

function renderVariant(artifact, screenshotByArtifact) {
  const key = `${artifact.caseId}/${artifact.variant}`;
  const screenshots = screenshotByArtifact.get(key) ?? [];
  const relHtml = relativeFromReview(artifact.htmlPath);
  const relRaw = relativeFromReview(artifact.rawPath);
  const relPrompt = relativeFromReview(artifact.promptPath);
  const consoleErrors = screenshots.flatMap((shot) => shot.consoleErrors ?? []);
  const outputKind = artifact.metadata.outputKind ?? "unknown";
  const statusClass = outputKind === "html" || outputKind === "repaired-html" ? "html" : "non-html";
  return `<article class="variant">
    <div class="variant-head">
      <div class="variant-title">
        <span>${escapeHtml(artifact.variant)}</span>
        <span class="status ${escapeAttr(statusClass)}">${escapeHtml(outputKind)}</span>
      </div>
      <div class="meta">${escapeHtml(artifact.metadata.modelProvider ?? "")}${artifact.metadata.model ? ` · ${escapeHtml(artifact.metadata.model)}` : ""}</div>
      <div class="skill">${escapeHtml(artifact.metadata.skillName ?? "")} · ${escapeHtml(artifact.metadata.skillPath ?? "")}</div>
      <div class="links">
        <a href="${escapeAttr(relHtml)}" target="_blank" rel="noreferrer">Open HTML</a>
        <a href="${escapeAttr(relRaw)}" target="_blank" rel="noreferrer">Raw output</a>
        <a href="${escapeAttr(relPrompt)}" target="_blank" rel="noreferrer">Prompt</a>
      </div>
    </div>
    <div class="shots">
      ${
        screenshots.length
          ? screenshots.map(renderScreenshot).join("\n")
          : '<div class="empty">No screenshots captured.</div>'
      }
    </div>
    ${consoleErrors.length ? `<div class="console-errors">Console errors: ${escapeHtml(consoleErrors.join(" | "))}</div>` : ""}
  </article>`;
}

function renderScreenshot(shot) {
  return `<figure>
    <figcaption>${escapeHtml(shot.viewport)} · ${shot.width}x${shot.height}</figcaption>
    <a href="${escapeAttr(shot.relativePath)}" target="_blank" rel="noreferrer"><img src="${escapeAttr(shot.relativePath)}" alt="${escapeAttr(`${shot.caseId} ${shot.variant} ${shot.viewport}`)}"></a>
  </figure>`;
}

function relativeFromReview(file) {
  return path.relative(reviewDir, file).split(path.sep).join("/");
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function escapeAttr(value) {
  return escapeHtml(value).replaceAll("'", "&#39;");
}
