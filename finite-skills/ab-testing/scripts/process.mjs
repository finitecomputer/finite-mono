import { existsSync } from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const harnessRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export function binPath(name) {
  const executable = process.platform === "win32" ? `${name}.cmd` : name;
  return path.join(harnessRoot, "node_modules/.bin", executable);
}

export async function runBin(name, args, options = {}) {
  const command = binPath(name);
  if (!existsSync(command)) {
    throw new Error(`Missing ${name}. Run \`pnpm install --frozen-lockfile\` in ${harnessRoot} first.`);
  }
  await run(command, args, options);
}

export async function run(command, args, options = {}) {
  await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? harnessRoot,
      env: { ...process.env, ...(options.env ?? {}) },
      stdio: "inherit",
    });
    child.on("error", reject);
    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`${path.basename(command)} exited with ${signal ?? code}`));
    });
  });
}
