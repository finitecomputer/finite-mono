import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { promisify } from "node:util";

import { repoRoot, workspaceRoot } from "@/lib/workspace-paths";

const execFileAsync = promisify(execFile);

export type PublishedEndpoint = {
  hostname: string;
  label: string;
  target_port: number | null;
  status: "reserved" | "published";
  run_command: string | null;
  run_cwd: string | null;
  desired_process_state: "external" | "running" | "stopped";
  auth: {
    mode: "self" | "emails" | "org" | "public";
    owner_email?: string;
    emails?: string[];
    org_domain?: string;
  };
  created_at: string;
  updated_at: string;
};

export type RuntimeImageRevision = {
  base_image?: string;
  image?: string;
  image_name?: string;
  image_tag?: string;
  store_path?: string;
};

type ControlPlaneDump = {
  admins: string[];
  invites: Array<{
    machineId: string;
    email: string;
    displayName: string;
    claimToken: string;
    createdAt: string;
    claimedAt?: string;
  }>;
  workloads: Array<{
    id: string;
    owner: string;
    owner_email?: string;
    namespace: string;
    runtime_profile: string;
    home_volume_size: string;
    opencode: {
      port: number;
      hostname: string;
      project_dir: string;
      auth?: {
        mode?: "self" | "emails" | "org" | "public";
        owner_email?: string;
        emails?: string[];
        org_domain?: string;
      };
    };
    ssh: {
      enable: boolean;
      node_port?: number;
    };
  }>;
  publishedEndpoints?: Array<PublishedEndpoint & { machineId: string }>;
  runtimeImageRevisions?: Record<string, RuntimeImageRevision>;
};

function controlPlaneRoot() {
  return process.env.FC_CONTROL_PLANE_ROOT || path.join(repoRoot(), ".fc-control-plane-dev");
}

function binaryOnPath(name: string) {
  const pathEntries = (process.env.PATH || "").split(path.delimiter).filter(Boolean);
  for (const entry of pathEntries) {
    const candidate = path.join(entry, name);
    if (existsSync(candidate)) {
      return candidate;
    }
  }
  return null;
}

function finitedBin() {
  if (process.env.FC_FINITED_BIN && existsSync(process.env.FC_FINITED_BIN)) {
    return path.resolve(process.env.FC_FINITED_BIN);
  }

  const pathBin = binaryOnPath("finited");
  if (pathBin) {
    return pathBin;
  }

  const debugBin = path.join(repoRoot(), "target", "debug", "finited");
  if (existsSync(debugBin)) {
    return debugBin;
  }

  const releaseBin = path.join(repoRoot(), "target", "release", "finited");
  if (existsSync(releaseBin)) {
    return releaseBin;
  }

  return null;
}

async function runFinited<T>(
  command: string,
  options?: {
    payload?: unknown;
    includeRepoRoot?: boolean;
  }
) {
  const finited = finitedBin();
  if (!finited) {
    throw new Error("finited binary not found");
  }

  const args = ["--control-plane-root", controlPlaneRoot(), "--workspace-root", workspaceRoot()];

  if (options?.includeRepoRoot) {
    args.push("--repo-root", repoRoot());
  }

  args.push(command);

  if (options?.payload !== undefined) {
    args.push("--payload", JSON.stringify(options.payload));
  }

  const { stdout } = await execFileAsync(finited, args, {
    encoding: "utf8",
    maxBuffer: 10 * 1024 * 1024,
  });

  return JSON.parse(stdout) as T;
}

async function runControlPlane<T>(
  command: string,
  options?: {
    payload?: unknown;
    includeRepoRoot?: boolean;
  }
) {
  return runFinited<T>(command, options);
}

async function ensureInitialized() {
  await runControlPlane<{ ok: boolean; bootstrapped: boolean; machineCount: number }>("init", {
    includeRepoRoot: true,
  });
}

export async function loadControlPlaneDump() {
  await ensureInitialized();
  return runControlPlane<ControlPlaneDump>("dump-state", {
    includeRepoRoot: true,
  });
}
