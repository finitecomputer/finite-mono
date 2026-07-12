import { DashboardShell } from "@/components/dashboard-shell";
import {
  coreProductProjects,
  coreProjectLabel,
  coreProjectPrimaryUrl,
  coreProjectRuntimeId,
  loadCoreMe,
} from "@/lib/core-client";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";

export default async function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const [viewer, core] = await Promise.all([
    loadOptionalViewerContext(),
    loadCoreMe({ cacheMode: "swr" }),
  ]);
  const machineIds = new Set<string>();
  const machines = [
    ...coreProductProjects(core.me?.projects ?? []).flatMap((project) => {
      const runtimeId = coreProjectRuntimeId(project);
      if (!runtimeId || machineIds.has(runtimeId)) {
        return [];
      }
      machineIds.add(runtimeId);
      return [
        {
          id: runtimeId,
          ownerLabel: coreProjectLabel(project),
          siteUrl: coreProjectPrimaryUrl(project) ?? undefined,
        },
      ];
    }),
  ];

  return (
    <DashboardShell
      isAdmin={viewer.isAdmin}
      machines={machines}
      saasMode={core.configured}
      viewerEmail={viewer.email}
    >
      {children}
    </DashboardShell>
  );
}
