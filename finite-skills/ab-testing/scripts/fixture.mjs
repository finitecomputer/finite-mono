// Local fixture prototype for FIN-149. Reuses the existing HTML A/B runner.
import { createHash } from "node:crypto";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const baselineFile = path.resolve(harnessRoot, "../skills/software-development/website-building-finite/SKILL.md");

export function prepareFixture({ fixtureFile, candidateFile, runsDir = path.join(harnessRoot, "runs/fixtures") }) {
  const fixture = JSON.parse(readFileSync(fixtureFile, "utf8"));
  if (fixture.version !== 1 || !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(fixture.id ?? "")) {
    throw new Error("Fixture needs version 1 and a lowercase, hyphen-separated id.");
  }
  for (const field of ["title", "brief", "scope"]) {
    if (typeof fixture[field] !== "string" || !fixture[field].trim()) throw new Error(`Missing fixture ${field}`);
  }
  for (const field of ["requiredVisibleText", "reviewCriteria"]) {
    if (!Array.isArray(fixture[field]) || !fixture[field].length || fixture[field].some((v) => typeof v !== "string" || !v.trim())) {
      throw new Error(`Fixture ${field} must be a non-empty list of text.`);
    }
  }
  const baseline = readFileSync(baselineFile, "utf8");
  const candidate = readFileSync(candidateFile ?? baselineFile, "utf8");
  if (!candidate.trim()) throw new Error("Candidate skill is empty.");
  mkdirSync(runsDir, { recursive: true });
  const runDir = mkdtempSync(path.join(runsDir, `${fixture.id}-`));
  const variants = [baseline, candidate].map((skill, index) => {
    const variant = index === 0 ? "skill-a" : "skill-b";
    const dir = path.join(runDir, "skills", variant);
    // Both variants get the same supporting files. Only SKILL.md changes.
    cpSync(path.dirname(baselineFile), dir, { recursive: true });
    const skillPath = path.join(dir, "SKILL.md");
    writeFileSync(skillPath, skill);
    return { variant, skillPath, sha256: createHash("sha256").update(skill).digest("hex") };
  });
  const configPath = path.join(runDir, "promptfooconfig.json");
  const config = {
    description: `Fixture prototype: ${fixture.title}`,
    prompts: ["{{brief}}"],
    providers: variants.map(({ variant, skillPath }) => ({
      id: pathToFileURL(path.join(harnessRoot, "providers/web-design-skill-provider.mjs")).href,
      label: variant,
      config: { variant, skillPath, runner: "devfinity", outputDir: path.join(runDir, "artifacts"), repairHtml: false },
    })),
    tests: [{ description: fixture.title, vars: { caseId: fixture.id, title: fixture.title, brief: fixture.brief } }],
  };
  const plan = {
    runId: path.basename(runDir), status: "prepared", createdAt: new Date().toISOString(),
    comparison: baseline === candidate ? "A/A control — identical skill text" : "A/B skill-text change",
    fixture, variants,
    limitations: "One selected skill per Runtime, HTML output only. Runtime/model versions are not pinned by this prototype. Timing in metadata includes runtime setup and cleanup.",
  };
  writeJson(path.join(runDir, "fixture.json"), fixture);
  writeJson(path.join(runDir, "plan.json"), plan);
  writeJson(configPath, config);
  return { runDir, configPath, plan };
}

function writeJson(file, value) {
  writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function main() {
  const { values } = parseArgs({ options: {
    fixture: { type: "string", default: "fixtures/coffee-shop-homepage.json" },
    candidate: { type: "string" },
  } });
  // These existing overrides would defeat the recorded skill snapshots or runner.
  for (const name of ["SKILL_AB_SKILL_A_PATH", "SKILL_AB_SKILL_B_PATH", "SKILL_AB_SKILL_A_SOURCE_PATH", "SKILL_AB_SKILL_B_SOURCE_PATH"]) {
    if (process.env[name]) throw new Error(`Unset ${name}; this command uses its saved skill snapshots.`);
  }
  const { runDir, configPath, plan } = prepareFixture({
    fixtureFile: path.resolve(harnessRoot, values.fixture),
    candidateFile: values.candidate ? path.resolve(harnessRoot, values.candidate) : undefined,
  });
  console.log(`${plan.comparison}\nPrepared: ${runDir}`);
  console.log(`Saved config: ${configPath}\nNo Agent or model call was made.`);
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  main();
}
