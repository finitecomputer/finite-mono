import { promises as fs } from "node:fs";
import path from "node:path";

type EnvLine =
  | { kind: "raw"; value: string }
  | { kind: "entry"; key: string; value: string };

export function secretsRoot() {
  return process.env.FC_SECRETS_ROOT || "/fc-secrets";
}

export function hermesEnvPath(machineId: string) {
  return path.join(/* turbopackIgnore: true */ secretsRoot(), machineId, "hermes.env");
}

export function sharedHermesEnvPath() {
  return path.join(/* turbopackIgnore: true */ secretsRoot(), "shared", "hermes.env");
}

export function machineSecretsDir(machineId: string) {
  return path.join(/* turbopackIgnore: true */ secretsRoot(), machineId);
}

function parseEnvFile(text: string) {
  return text.split("\n").map<EnvLine>((line) => {
    const match = line.match(/^\s*([A-Za-z_][A-Za-z0-9_]*)=(.*)$/);
    if (!match) {
      return { kind: "raw", value: line };
    }

    return {
      kind: "entry",
      key: match[1],
      value: match[2],
    };
  });
}

function renderEnvFile(lines: EnvLine[]) {
  return `${lines
    .map((line) => (line.kind === "raw" ? line.value : `${line.key}=${line.value}`))
    .join("\n")
    .replace(/\n+$/, "")}\n`;
}

async function readEnvLines(filePath: string) {
  try {
    const text = await fs.readFile(filePath, "utf8");
    return parseEnvFile(text);
  } catch {
    return [] as EnvLine[];
  }
}

export async function readEnvMap(filePath: string) {
  const lines = await readEnvLines(filePath);
  const values = new Map<string, string>();

  for (const line of lines) {
    if (line.kind === "entry") {
      values.set(line.key, line.value);
    }
  }

  return values;
}

export async function readMachineHermesEnvMap(machineId: string) {
  return readEnvMap(hermesEnvPath(machineId));
}

export async function readSharedHermesEnvMap() {
  return readEnvMap(sharedHermesEnvPath());
}

async function writeEnvValues(filePath: string, updates: Record<string, string | null>) {
  await fs.mkdir(path.dirname(filePath), { recursive: true });

  const lines = await readEnvLines(filePath);
  const remaining = new Map(Object.entries(updates));
  const nextLines: EnvLine[] = [];

  for (const line of lines) {
    if (line.kind !== "entry") {
      nextLines.push(line);
      continue;
    }

    if (!remaining.has(line.key)) {
      nextLines.push(line);
      continue;
    }

    const nextValue = remaining.get(line.key);
    remaining.delete(line.key);

    if (nextValue == null || nextValue === "") {
      continue;
    }

    nextLines.push({
      kind: "entry",
      key: line.key,
      value: nextValue,
    });
  }

  for (const [key, value] of remaining.entries()) {
    if (value == null || value === "") {
      continue;
    }

    nextLines.push({
      kind: "entry",
      key,
      value,
    });
  }

  await fs.writeFile(filePath, renderEnvFile(nextLines), "utf8");
}

export async function writeMachineHermesEnvValues(
  machineId: string,
  updates: Record<string, string | null>
) {
  await writeEnvValues(hermesEnvPath(machineId), updates);
}

export async function writeSharedHermesEnvValues(updates: Record<string, string | null>) {
  await writeEnvValues(sharedHermesEnvPath(), updates);
}
