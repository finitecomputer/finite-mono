import {
  coreProductProjectForLegacyMachineId,
  coreProductProjectForRouteId,
  coreProjectLabel,
  coreProjectPrimaryUrl,
  coreProjectSupportsRetirement,
  loadCoreMe,
  resolveCoreRuntimeRoute,
  type CoreReadCacheMode,
  type CoreVisibleProject,
  type CoreMe,
} from "@/lib/core-client";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";

type ViewerContext = Awaited<ReturnType<typeof import("@/lib/dashboard-auth").loadOptionalViewerContext>>;

export type DashboardMachineAccess = {
  viewer: ViewerContext;
  coreProject: CoreVisibleProject;
  mode: "core";
  /** Stable Agent Runtime id used by every browser route and navigation link. */
  machineId: string;
  displayName: string;
  primaryUrl: string | null;
  canRetireRuntime: boolean;
};

type DashboardMachineAccessOptions = {
  coreCacheMode?: CoreReadCacheMode;
};

export async function loadDashboardMachineAccess(
  routeIdentifier: string,
  options: DashboardMachineAccessOptions = {}
): Promise<DashboardMachineAccess | null> {
  const viewer = await loadOptionalViewerContext();
  let core = await loadCoreMe({ cacheMode: options.coreCacheMode });
  let coreProject = await projectForRouteIdentifier(core.me, routeIdentifier);
  if (!coreProject && options.coreCacheMode === "swr") {
    core = await loadCoreMe();
    coreProject = await projectForRouteIdentifier(core.me, routeIdentifier);
  }
  const runtime = coreProject?.runtime;
  if (!coreProject || !runtime) return null;

  return {
    viewer,
    coreProject,
    mode: "core",
    machineId: runtime.id,
    displayName: coreProjectLabel(coreProject),
    primaryUrl: coreProjectPrimaryUrl(coreProject),
    canRetireRuntime: coreProjectSupportsRetirement(coreProject),
  };
}

async function projectForRouteIdentifier(
  me: CoreMe | null,
  routeIdentifier: string
) {
  const projects = me?.projects ?? [];
  const stableProject = coreProductProjectForRouteId(projects, routeIdentifier);
  if (stableProject) return stableProject;

  // During an N-1 rollout, an older Core response can still carry the former
  // source-machine field. Use it only on this server-side compatibility path.
  const legacyProject = coreProductProjectForLegacyMachineId(projects, routeIdentifier);
  if (legacyProject) return legacyProject;

  const resolution = await resolveCoreRuntimeRoute(routeIdentifier);
  if (!resolution) return null;
  return (
    coreProductProjectForRouteId(projects, resolution.runtime_id) ??
    coreProductProjectForRouteId(projects, resolution.project_id)
  );
}

export function coreProjectOverviewHref(project: CoreVisibleProject) {
  const runtimeId = project.runtime?.id.trim();
  return runtimeId ? `/dashboard/machines/${encodeURIComponent(runtimeId)}` : null;
}
