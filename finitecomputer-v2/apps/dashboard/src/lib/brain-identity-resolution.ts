export type BrainIdentityResolution = {
  state: "resolved" | "notShared" | "unknown" | "ambiguous" | "unavailable";
  reason: "noParticipation" | null;
};

/** Safe fixed copy only. notShared deliberately hides whether Core has an
 * Account association, so it must never be described as a missing link. */
export function unresolvedIdentityMessage(resolution?: BrainIdentityResolution | null): string {
  if (resolution?.state === "notShared" && resolution.reason === "noParticipation") {
    return "This key has not acted in this Brain yet.";
  }
  switch (resolution?.state) {
    case "notShared": return "Identity details are not available to this Brain.";
    case "unknown": return "No supported identity details are available.";
    case "ambiguous": return "Identity records conflict.";
    case "unavailable": return "Identity lookup is unavailable. Try again later.";
    default: return "Identity details are unavailable.";
  }
}
