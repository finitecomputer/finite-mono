import {
  loadCoreMe,
  loadCoreSourceHostRelayEndpoint,
  coreProductProjectForMachineId,
  type CoreAgentCreationRequestSummary,
  type CoreReadCacheMode,
  type CoreReadOptions,
  type CoreVisibleProject,
} from "@/lib/core-client";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";
import {
  relayEndpointForSourceHost,
  type RelayEndpointConfig,
} from "@/lib/finite-relay-client";

type ViewerContext = Awaited<ReturnType<typeof import("@/lib/dashboard-auth").loadOptionalViewerContext>>;

export type DashboardMachineAccess = {
  viewer: ViewerContext;
  coreProject: CoreVisibleProject | null;
  mode: "core";
  machineId: string;
  displayName: string;
  ownerLabel: string | null;
  primaryUrl: string | null;
  publishedAppUrls: string[];
  relayEndpoint: RelayEndpointConfig | null;
  canRemoveKataRuntime: boolean;
};

type DashboardMachineAccessOptions = {
  coreCacheMode?: CoreReadCacheMode;
};

export async function loadDashboardMachineAccess(
  machineId: string,
  options: DashboardMachineAccessOptions = {}
): Promise<DashboardMachineAccess | null> {
  const viewer = await loadOptionalViewerContext();
  let core = await loadCoreMe({
    cacheMode: options.coreCacheMode,
  });
  let coreProject = coreProductProjectForMachineId(core.me?.projects ?? [], machineId);
  if (!coreProject && options.coreCacheMode === "swr") {
    core = await loadCoreMe();
    coreProject = coreProductProjectForMachineId(core.me?.projects ?? [], machineId);
  }
  const runtime = coreProject?.runtime;
  if (!coreProject || !runtime) {
    return null;
  }

  return {
    viewer,
    coreProject,
    mode: "core",
    machineId: runtime.source_machine_id,
    displayName:
      coreProject.project.display_name.trim() ||
      runtime.host_facts.display_name.trim() ||
      runtime.source_machine_id,
    ownerLabel: runtime.source_host_id,
    primaryUrl: firstSafeHttpUrl(runtime.host_facts.published_app_urls),
    publishedAppUrls: runtime.host_facts.published_app_urls.filter(safeHttpUrl),
    relayEndpoint: await relayEndpointForCoreProject(coreProject, {
      cacheMode: options.coreCacheMode,
    }),
    canRemoveKataRuntime: coreProjectHasRunningKataCreationRequest(
      coreProject,
      core.me?.agent_creation_requests ?? []
    ),
  };
}

export function coreProjectHasRunningKataCreationRequest(
  project: CoreVisibleProject,
  requests: CoreAgentCreationRequestSummary[]
) {
  return requests.some(
    (request) =>
      request.project_id === project.project.id &&
      request.agent_runtime_id === project.runtime?.id &&
      request.status === "running" &&
      request.runner_class === "kata"
  );
}

export async function relayEndpointForCoreProject(
  project: CoreVisibleProject | null | undefined,
  options: CoreReadOptions = {}
): Promise<RelayEndpointConfig | null> {
  const sourceHostId = project?.runtime?.source_host_id;
  if (!sourceHostId) {
    return null;
  }
  const coreEndpoint = await loadCoreSourceHostRelayEndpoint(sourceHostId, {
    cacheMode: options.cacheMode,
  });
  if (coreEndpoint) {
    return {
      baseUrl: coreEndpoint.url,
      adminToken: coreEndpoint.admin_token,
    };
  }
  return relayEndpointForSourceHost(sourceHostId);
}

export function coreProjectOverviewHref(project: CoreVisibleProject) {
  const machineId = project.runtime?.source_machine_id?.trim();
  return machineId ? `/dashboard/machines/${encodeURIComponent(machineId)}` : null;
}

function firstSafeHttpUrl(values: string[]) {
  return values.find(safeHttpUrl) ?? null;
}

function safeHttpUrl(value: string) {
  try {
    const parsed = new URL(value);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}
