import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { run } from "./process.mjs";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const reviewPath = path.join(harnessRoot, "runs/latest/review/index.html");

if (!existsSync(reviewPath)) {
  throw new Error(`No review page found at ${reviewPath}. Run \`pnpm run ab\` first.`);
}

if (process.platform === "darwin") {
  await run("open", [reviewPath], { cwd: harnessRoot });
} else if (process.platform === "linux") {
  await run("xdg-open", [reviewPath], { cwd: harnessRoot });
} else {
  console.log(reviewPath);
}
