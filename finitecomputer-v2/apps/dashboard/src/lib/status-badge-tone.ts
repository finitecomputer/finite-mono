import type { StatusBadgeState } from "@/lib/design-tokens";
import { statusBadgeClassName } from "@/lib/design-tokens";

export type { StatusBadgeState };

export function statusBadgeToneClass(status: StatusBadgeState) {
  return statusBadgeClassName(status);
}
