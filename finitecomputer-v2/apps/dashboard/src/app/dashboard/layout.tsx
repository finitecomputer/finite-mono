import { DashboardShell } from "@/components/dashboard-shell";
import { coreProjectLabel, coreProjectMachineId, loadCoreMe } from "@/lib/core-client";
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
    ...(core.me?.projects ?? []).flatMap((project) => {
      const machineId = coreProjectMachineId(project);
      if (!machineId || machineIds.has(machineId)) {
        return [];
      }
      machineIds.add(machineId);
      return [
        {
          id: machineId,
          ownerLabel: coreProjectLabel(project),
          siteUrl: project.runtime?.host_facts.published_app_urls[0],
        },
      ];
    }),
  ];

  return (
    <DashboardShell
      isAdmin={viewer.isAdmin}
      machines={machines}
      saasMode={core.configured && !viewer.isAdmin}
      viewerEmail={viewer.email}
    >
      {children}
    </DashboardShell>
  );
}
