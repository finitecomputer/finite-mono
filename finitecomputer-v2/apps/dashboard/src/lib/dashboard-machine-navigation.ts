import type { CoreRuntimeStatus } from "./core-client";

export type MachineNavItem = {
  id: string;
  ownerLabel: string;
  runtimeStatus: CoreRuntimeStatus;
  siteUrl?: string;
  nativeHermesChat?: boolean;
};

export function activeNavigationMachine(
  machines: MachineNavItem[],
  selectedMachineId: string | null,
  activeMachineId: string | null,
  saasMode: boolean,
): MachineNavItem | null {
  // Layout props can still contain the pre-launch fleet after signup. The
  // authenticated machine page owns route access; this fallback only supplies
  // navigation chrome until the layout receives the updated fleet. A query
  // parameter alone must not select an unknown agent.
  return machines.find((machine) => machine.id === selectedMachineId)
    ?? (saasMode && activeMachineId
      ? { id: activeMachineId, ownerLabel: "Your agent", runtimeStatus: "unknown" }
      : null);
}
