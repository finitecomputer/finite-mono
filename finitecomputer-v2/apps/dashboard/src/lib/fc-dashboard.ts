import { promises as fs } from "node:fs";
import path from "node:path";

import type { PublishedEndpoint, RuntimeImageRevision } from "@/lib/control-plane";
import { loadControlPlaneDump } from "@/lib/control-plane";
import {
  defaultRuntimeProfileFor,
  runtimeBaseImageFor,
  runtimeImageFor,
  runtimeImageForProfile,
  runtimeProfileConfigFor,
  runtimeProfilesFor,
  type RuntimeProfileConfig,
} from "@/lib/runtime-profiles";
import { agentClusterRoot, clusterConfigPath, repoRoot } from "@/lib/workspace-paths";

export type SiteAuthMode = "self" | "emails" | "org" | "public";
export type StatusBadgeState = "pending" | "in_progress" | "complete" | "blocked";
export type { RuntimeProfileConfig } from "@/lib/runtime-profiles";

export type ClusterConfig = {
  enabled: boolean;
  cluster_name: string;
  base_domain: string;
  org_domain?: string;
  default_runtime_profile?: string;
  runtime_profiles?: Record<string, RuntimeProfileConfig>;
  letsencrypt?: {
    email?: string;
  };
  oauth2_proxy?: {
    enabled?: boolean;
    auth_host?: string;
    client_id?: string;
  };
  dashboard?: {
    admins?: string[];
    hostname?: string;
  };
  gitea?: {
    enabled?: boolean;
    hostname?: string;
  };
  admin_authorized_keys?: string[];
};

export type WorkloadAuthConfig = {
  mode?: SiteAuthMode;
  owner_email?: string;
  emails?: string[];
  org_domain?: string;
};

export type WorkloadFile = {
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
    auth?: WorkloadAuthConfig;
  };
  ssh: {
    enable: boolean;
    node_port?: number;
  };
};

export type InviteRecord = {
  machineId: string;
  email: string;
  displayName: string;
  claimToken: string;
  createdAt: string;
  claimedAt?: string;
};

export type DashboardState = {
  admins: string[];
  invites: InviteRecord[];
};

export type DeployMetadata = {
  source_rev?: string;
  runtime_image_revisions?: Record<string, RuntimeImageRevision>;
  active_runtime_image_revisions?: Record<string, RuntimeImageRevision>;
};

export type MachineRecord = {
  workload: WorkloadFile;
  invite: InviteRecord | null;
  ownerEmail: string | null;
  siteUrl: string;
  runtimeProfile: string;
  runtimeProfileLabel: string;
  runtimeImage: string;
  runtimeBaseImage: string;
  authMode: SiteAuthMode;
  authSummary: string;
  publishedEndpoints: PublishedEndpoint[];
};

async function readJsonFile<T>(filePath: string): Promise<T> {
  const raw = await fs.readFile(filePath, "utf8");
  return JSON.parse(raw) as T;
}

async function readJsonFileIfExists<T>(filePath: string): Promise<T | null> {
  try {
    const raw = await fs.readFile(filePath, "utf8");
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

async function loadRepoSourceRev() {
  const dotGitPath = path.join(repoRoot(), ".git");
  let gitDir = dotGitPath;

  try {
    const dotGit = (await fs.readFile(dotGitPath, "utf8")).trim();
    if (dotGit.startsWith("gitdir:")) {
      gitDir = path.resolve(repoRoot(), dotGit.replace(/^gitdir:\s*/u, ""));
    }
  } catch {
    // A normal checkout has .git as a directory, which is fine.
  }

  try {
    const head = (await fs.readFile(path.join(gitDir, "HEAD"), "utf8")).trim();
    if (/^[0-9a-f]{40}$/iu.test(head)) {
      return head;
    }

    const match = head.match(/^ref:\s+(.+)$/u);
    if (!match) {
      return null;
    }

    const refPath = path.join(gitDir, match[1]!);
    const ref = (await fs.readFile(refPath, "utf8")).trim();
    return /^[0-9a-f]{40}$/iu.test(ref) ? ref : null;
  } catch {
    return null;
  }
}

async function loadDeployMetadata() {
  const candidates = [
    process.env.FC_DEPLOY_METADATA_PATH,
    "/fc-host/agent-cluster/deploy-metadata.json",
    path.join(agentClusterRoot(), "deploy-metadata.json"),
  ].filter((candidate): candidate is string => Boolean(candidate));

  for (const candidate of candidates) {
    const metadata = await readJsonFileIfExists<DeployMetadata>(candidate);
    if (metadata) {
      return {
        ...metadata,
        source_rev: metadata.source_rev ?? await loadRepoSourceRev() ?? undefined,
      };
    }
  }

  const sourceRev = await loadRepoSourceRev();
  return sourceRev ? { source_rev: sourceRev } : null;
}

function authModeFor(workload: WorkloadFile): SiteAuthMode {
  return workload.opencode.auth?.mode ?? "self";
}

function ownerEmailFor(workload: WorkloadFile, invite: InviteRecord | null) {
  const auth = workload.opencode.auth;

  if (auth?.owner_email) {
    return auth.owner_email;
  }

  if (workload.owner_email) {
    return workload.owner_email;
  }

  if (auth?.mode === "emails" && auth.emails?.[0]) {
    return auth.emails[0];
  }

  return invite?.email ?? null;
}

function authSummaryFor(cluster: ClusterConfig, workload: WorkloadFile) {
  const auth = workload.opencode.auth ?? {};
  const mode = authModeFor(workload);

  if (mode === "self") {
    return auth.owner_email ?? workload.owner_email ?? "Only the invited user";
  }

  if (mode === "emails") {
    return (auth.emails ?? []).join(", ") || "Specific emails";
  }

  if (mode === "org") {
    return `Anyone at ${auth.org_domain ?? defaultOrgDomainFor(cluster)}`;
  }

  return "Public internet";
}

export async function loadDashboardModel() {
  const [cluster, dump, deployMetadata] = await Promise.all([
    readJsonFile<ClusterConfig>(clusterConfigPath()),
    loadControlPlaneDump(),
    loadDeployMetadata(),
  ]);
  const dashboardState = {
    admins: cluster.dashboard?.admins ?? dump.admins,
    invites: dump.invites,
  } satisfies DashboardState;
  const workloads = dump.workloads as WorkloadFile[];

  const inviteByMachineId = new Map(
    dashboardState.invites.map((invite) => [invite.machineId, invite])
  );
  const publishedEndpointsByMachineId = new Map<string, PublishedEndpoint[]>();

  for (const endpoint of dump.publishedEndpoints ?? []) {
    const existing = publishedEndpointsByMachineId.get(endpoint.machineId) ?? [];
    existing.push(endpoint);
    publishedEndpointsByMachineId.set(endpoint.machineId, existing);
  }

  const machines = workloads
    .map((workload) => {
      const invite = inviteByMachineId.get(workload.id) ?? null;
      const runtimeProfile = runtimeProfileConfigFor(cluster, workload.runtime_profile);

      return {
        workload,
        invite,
        ownerEmail: ownerEmailFor(workload, invite),
        siteUrl: `https://${workload.opencode.hostname}`,
        runtimeProfile: runtimeProfile.id,
        runtimeProfileLabel: runtimeProfile.config.label,
        runtimeImage: runtimeImageFor(
          dump.runtimeImageRevisions,
          workload,
          runtimeBaseImageFor(cluster, workload),
        ),
        runtimeBaseImage: runtimeBaseImageFor(cluster, workload),
        authMode: authModeFor(workload),
        authSummary: authSummaryFor(cluster, workload),
        publishedEndpoints: (publishedEndpointsByMachineId.get(workload.id) ?? []).sort((left, right) =>
          left.hostname.localeCompare(right.hostname)
        ),
      } satisfies MachineRecord;
    })
    .sort((left, right) => left.workload.id.localeCompare(right.workload.id));

  return {
    cluster,
    dashboardState,
    deployMetadata,
    runtimeImageRevisions: dump.runtimeImageRevisions ?? deployMetadata?.runtime_image_revisions ?? {},
    machines,
  };
}

function defaultOrgDomainFor(cluster: ClusterConfig) {
  return cluster.org_domain?.trim().toLowerCase() || cluster.base_domain;
}

export function formatDate(value?: string) {
  if (!value) {
    return "Not yet";
  }

  return new Intl.DateTimeFormat("en-US", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));
}

export function statusBadgeTone(status: StatusBadgeState) {
  switch (status) {
    case "complete":
      return "emerald";
    case "in_progress":
      return "amber";
    case "blocked":
      return "rose";
    default:
      return "zinc";
  }
}
export {
  defaultRuntimeProfileFor,
  runtimeBaseImageFor,
  runtimeImageForProfile,
  runtimeProfileConfigFor,
  runtimeProfilesFor,
};
