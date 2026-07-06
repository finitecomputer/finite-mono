import { existsSync, readdirSync } from "node:fs";
import path from "node:path";

function isWorkspaceRoot(candidate: string) {
  return existsSync(path.join(candidate, ".fc-workspace"));
}

export function repoRoot() {
  if (process.env.FC_REPO_ROOT) {
    return path.resolve(process.env.FC_REPO_ROOT);
  }

  return path.resolve(process.cwd(), "../..");
}

function discoverWorkspaceRoots(root: string) {
  const workspacesDir = path.join(root, "workspaces");
  if (!existsSync(workspacesDir)) {
    return [] as string[];
  }

  return readdirSync(workspacesDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => path.join(workspacesDir, entry.name))
    .filter(isWorkspaceRoot)
    .sort((left, right) => left.localeCompare(right));
}

export function workspaceRoot() {
  if (process.env.FC_WORKSPACE_ROOT) {
    return path.resolve(process.env.FC_WORKSPACE_ROOT);
  }

  const root = repoRoot();
  const explicitRel = process.env.FC_WORKSPACE_REL?.trim();
  if (explicitRel) {
    const candidate = path.resolve(root, explicitRel);
    if (isWorkspaceRoot(candidate)) {
      return candidate;
    }
  }

  const candidates = discoverWorkspaceRoots(root);
  if (candidates.length > 0) {
    return candidates[0];
  }

  return path.join(root, "workspaces");
}

export function agentClusterRoot() {
  return path.join(workspaceRoot(), "agent-cluster");
}

export function clusterConfigPath() {
  return path.join(agentClusterRoot(), "cluster.json");
}
