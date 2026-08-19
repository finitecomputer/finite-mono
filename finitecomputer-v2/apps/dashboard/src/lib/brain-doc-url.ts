/**
 * brain:// document URLs for the chat preview pane (viewer plan Phase 2).
 *
 * Format (rename-proof, grill #3): `brain://<brain-id>/<folder-id>/<path>`
 * — ids are stable identifiers; display names never appear in the URL.
 */

export type BrainDocRef = {
  brainId: string;
  folderId: string;
  path: string;
};

const ID_PATTERN = /^[a-z0-9][a-z0-9_-]{0,127}$/u;
const PATH_PATTERN = /^[^\s?#]+$/u;

/** Parse a brain:// URL into its document reference, or null if malformed. */
export function parseBrainDocUrl(value: string): BrainDocRef | null {
  if (!value.startsWith("brain://")) return null;
  const rest = value.slice("brain://".length);
  const [brainId, folderId, ...pathParts] = rest.split("/");
  if (!brainId || !folderId || pathParts.length === 0) return null;
  if (!ID_PATTERN.test(brainId) || !ID_PATTERN.test(folderId)) return null;
  const path = pathParts.join("/");
  if (!path || !PATH_PATTERN.test(path)) return null;
  return { brainId, folderId, path };
}

/** Render a document reference back into its brain:// URL. */
export function formatBrainDocUrl(doc: BrainDocRef): string {
  return `brain://${doc.brainId}/${doc.folderId}/${doc.path}`;
}
