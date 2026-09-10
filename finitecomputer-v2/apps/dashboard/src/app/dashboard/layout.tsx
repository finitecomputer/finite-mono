import { wasmSpikeEnabled, WASM_SPIKE_MACHINE } from "@/lib/wasm-spike";
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
  if (wasmSpikeEnabled()) {
    return <DashboardShell isAdmin={false} saasMode browserChat viewerEmail="local@wasm-spike.test"
      machines={[{ id: WASM_SPIKE_MACHINE, ownerLabel: "Hermes", runtimeStatus: "online" }]}>
      {children}
    </DashboardShell>;
  }
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
          runtimeStatus: project.runtime?.runtime_status ?? "unknown",
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
