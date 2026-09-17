/** Only the explicit local design fixture exposes unfinished product routes. */
export const dashboardDesignPreviewEnabled = process.env.NODE_ENV === "development"
  && process.env.NEXT_PUBLIC_FC_DESIGN_PREVIEWS === "1";
