import { mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { runBin } from "./process.mjs";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

mkdirSync(path.join(harnessRoot, "runs/latest/artifacts"), { recursive: true });

await runBin("promptfoo", ["eval", "-c", "promptfooconfig.yaml"], { cwd: harnessRoot });
