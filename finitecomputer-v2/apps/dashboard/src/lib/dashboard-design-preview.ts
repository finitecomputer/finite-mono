/** Only the explicit local design fixture exposes unfinished product routes. */
export const dashboardDesignPreviewEnabled = process.env.NODE_ENV === "development"
  && process.env.NEXT_PUBLIC_FC_DESIGN_PREVIEWS === "1";

/** Sample product data is confined to the local account and exact fixture agent. */
export function dashboardAgentDesignPreviewEnabled(machineId: string) {
  return dashboardDesignPreviewEnabled
    && process.env.FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH === "1"
    && machineId === "runtime_web_design";
}
