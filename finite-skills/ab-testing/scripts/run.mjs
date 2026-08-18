import { rmSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { run, runBin } from "./process.mjs";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const latestDir = path.join(harnessRoot, "runs/latest");

rmSync(latestDir, { recursive: true, force: true });
mkdirSync(path.join(latestDir, "artifacts"), { recursive: true });

await run(process.execPath, ["scripts/eval.mjs"], { cwd: harnessRoot });
await runBin("playwright", ["test", "-c", "playwright.config.mjs"], { cwd: harnessRoot });
await run(process.execPath, ["scripts/build-review.mjs"], { cwd: harnessRoot });

console.log("");
console.log(`Review page: ${path.join(latestDir, "review/index.html")}`);
console.log("Run `pnpm run open` from finite-skills/ab-testing to open it.");
