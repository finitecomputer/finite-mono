import { mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { runBin } from "./process.mjs";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const configPath = process.env.SKILL_AB_CONFIG || "promptfooconfig.yaml";
const args = ["eval", "-c", configPath];
const defaultMaxConcurrency = "2";

const runner = String(process.env.SKILL_AB_RUNNER || "devfinity").trim().toLowerCase();
const promptfooEvalTimeoutMs = resolvePromptfooEvalTimeoutMs(runner);
if (process.env.SKILL_AB_MAX_CONCURRENCY) {
  args.push("--max-concurrency", process.env.SKILL_AB_MAX_CONCURRENCY);
} else if (runner === "devfinity") {
  args.push("--max-concurrency", defaultMaxConcurrency);
}

mkdirSync(path.join(harnessRoot, "runs/latest/artifacts"), { recursive: true });

await runBin("promptfoo", args, {
  cwd: harnessRoot,
  env: { PROMPTFOO_EVAL_TIMEOUT_MS: promptfooEvalTimeoutMs },
});

function resolvePromptfooEvalTimeoutMs(runner) {
  if (process.env.PROMPTFOO_EVAL_TIMEOUT_MS) {
    return process.env.PROMPTFOO_EVAL_TIMEOUT_MS;
  }
  if (process.env.SKILL_AB_PROMPTFOO_EVAL_TIMEOUT_MS) {
    return process.env.SKILL_AB_PROMPTFOO_EVAL_TIMEOUT_MS;
  }
  const provider = String(process.env.SKILL_AB_PROVIDER || "finite-private").trim().toLowerCase();
  if (runner === "provider" && provider === "finite-private") {
    return String(process.env.SKILL_AB_FINITE_PRIVATE_TIMEOUT_MS || process.env.SKILL_AB_TIMEOUT_MS || 20 * 60 * 1000);
  }
  if (runner === "devfinity") {
    return String(process.env.SKILL_AB_DEVFINITY_TIMEOUT_MS || 30 * 60 * 1000);
  }
  return String(process.env.SKILL_AB_TIMEOUT_MS || 10 * 60 * 1000);
}
