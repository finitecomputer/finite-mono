import { mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { runBin } from "./process.mjs";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const configPath = process.env.SKILL_AB_CONFIG || "promptfooconfig.yaml";
const args = ["eval", "-c", configPath];

if (process.env.SKILL_AB_MAX_CONCURRENCY) {
  args.push("--max-concurrency", process.env.SKILL_AB_MAX_CONCURRENCY);
}

mkdirSync(path.join(harnessRoot, "runs/latest/artifacts"), { recursive: true });

await runBin("promptfoo", args, { cwd: harnessRoot });
