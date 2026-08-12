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
for (const screenshot of screenshots) {
  const key = `${screenshot.caseId}/${screenshot.variant}`;
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
      const metadata = readJson(path.join(dir, "metadata.json"), {});
      items.push({
        caseId,
        caseTitle: metadata.caseTitle ?? caseId,
        htmlPath: path.join(dir, "index.html"),
        metadata,
        promptPath: path.join(dir, "prompt.txt"),
        rawPath: path.join(dir, "output.raw.md"),
        variant,
      });
    }
  }
  return items;
}

function groupByCase(artifacts) {
  const cases = new Map();
  for (const artifact of artifacts) {
    const existing = cases.get(artifact.caseId) ?? {
      caseId: artifact.caseId,
      caseTitle: artifact.caseTitle,
      variants: [],
    };
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
    .case { border-top: 1px solid var(--line); padding: 26px 0 34px; }
    .case:first-child { border-top: 0; }
    .case-head { display: flex; align-items: end; justify-content: space-between; gap: 16px; margin-bottom: 14px; }
    h2 { margin: 0; font-size: 18px; }
    .winner { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; color: var(--muted); font-size: 13px; }
    .winner label { display: inline-flex; gap: 6px; align-items: center; padding: 7px 9px; background: var(--paper); border: 1px solid var(--line); border-radius: 6px; color: var(--ink); }
    .variants { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 16px; }
    .variant { background: var(--paper); border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
    .variant-head { padding: 14px 14px 12px; border-bottom: 1px solid var(--line); display: grid; gap: 6px; }
    .variant-title { display: flex; justify-content: space-between; gap: 10px; align-items: center; font-weight: 760; }
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
    ${
      cases.length
        ? cases.map((item) => renderCase(item, screenshotByArtifact)).join("\n")
        : '<div class="empty">No artifacts found. Run the Promptfoo eval first.</div>'
    }
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
  </script>
</body>
</html>`;
}

function renderCase(item, screenshotByArtifact) {
  return `<section class="case">
    <div class="case-head">
      <div>
        <h2>${escapeHtml(item.caseTitle)}</h2>
        <div class="meta">${escapeHtml(item.caseId)}</div>
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
  return `<article class="variant">
    <div class="variant-head">
      <div class="variant-title">
        <span>${escapeHtml(artifact.variant)}</span>
        <span class="meta">${escapeHtml(artifact.metadata.model ?? "")}</span>
      </div>
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
