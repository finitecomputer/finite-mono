import type { CoreRuntimeStatus } from "@/lib/core-client";

type DashboardMachineStatusPresentation = {
  className: `is-${CoreRuntimeStatus}`;
  label: string;
};

export function dashboardMachineStatusPresentation(
  status: CoreRuntimeStatus
): DashboardMachineStatusPresentation {
  if (status === "online") {
    return { className: "is-online", label: "Online" };
  }
  if (status === "offline") {
    return { className: "is-offline", label: "Offline" };
  }
  if (status === "stale") {
    return { className: "is-stale", label: "Needs attention" };
  }
  return { className: "is-unknown", label: "Status unknown" };
}
