export type ConnectionsReadiness = "loading" | "error" | "ready";

/**
 * Connections status is returned only after the server has completed the
 * existing typed owner claim. Until then the page must not expose controls.
 */
export function connectionsReadiness(
  hasClaimedStatus: boolean,
  error: string | null
): ConnectionsReadiness {
  if (hasClaimedStatus) {
    return "ready";
  }
  return error ? "error" : "loading";
}
