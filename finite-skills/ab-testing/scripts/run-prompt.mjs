import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { run, runBin } from "./process.mjs";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const latestDir = path.join(harnessRoot, "runs/latest");
const generatedConfig = path.join(latestDir, "promptfooconfig.generated.yaml");
const providerId = pathToFileURL(path.join(harnessRoot, "providers/web-design-skill-provider.mjs")).href;

const brief = await readBrief();
const title = process.env.SKILL_AB_CASE_TITLE || titleFromBrief(brief);
const caseId = sanitize(process.env.SKILL_AB_CASE_ID || title);

rmSync(latestDir, { recursive: true, force: true });
mkdirSync(path.join(latestDir, "artifacts"), { recursive: true });
writeFileSync(generatedConfig, renderConfig({ brief, caseId, title }), "utf8");

await run(process.execPath, ["scripts/eval.mjs"], {
  cwd: harnessRoot,
  env: { SKILL_AB_CONFIG: generatedConfig },
});
await runBin("playwright", ["test", "-c", "playwright.config.mjs"], { cwd: harnessRoot });
await run(process.execPath, ["scripts/build-review.mjs"], { cwd: harnessRoot });

console.log("");
console.log(`Review page: ${path.join(latestDir, "review/index.html")}`);
console.log("Run `pnpm run open` from finite-skills/ab-testing to open it.");

async function readBrief() {
  const args = process.argv.slice(2);
  if (args[0] === "--") {
    args.shift();
  }
  const argvBrief = args.join(" ").trim();
  if (argvBrief) {
    return argvBrief;
  }
  if (!process.stdin.isTTY) {
    const chunks = [];
    for await (const chunk of process.stdin) {
      chunks.push(chunk);
    }
    const stdinBrief = Buffer.concat(chunks).toString("utf8").trim();
    if (stdinBrief) {
      return stdinBrief;
    }
  }
  throw new Error('Provide a build prompt, for example: pnpm run ab:prompt -- "Build a pricing page for..."');
}

function renderConfig({ brief, caseId, title }) {
  return `description: Finite skill A/B web-design spot check

prompts:
  - "{{brief}}"

providers:
  - id: ${yamlScalar(providerId)}
    label: skill-a website-building-finite
    config:
      variant: skill-a
      skillPath: ../skills/software-development/website-building-finite/SKILL.md
      outputDir: ./runs/latest/artifacts
      maxOutputTokens: 6000
  - id: ${yamlScalar(providerId)}
    label: skill-b impeccable-finite
    config:
      variant: skill-b
      skillPath: ../skills/software-development/impeccable-finite/SKILL.md
      outputDir: ./runs/latest/artifacts
      maxOutputTokens: 6000

tests:
  - description: ${yamlScalar(title)}
    vars:
      caseId: ${yamlScalar(caseId)}
      title: ${yamlScalar(title)}
      brief: |-
${indent(brief, 8)}
`;
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

function yamlScalar(value) {
  return JSON.stringify(String(value));
}

function indent(value, spaces) {
  const prefix = " ".repeat(spaces);
  return String(value)
    .split(/\r?\n/)
    .map((line) => `${prefix}${line}`)
    .join("\n");
}
