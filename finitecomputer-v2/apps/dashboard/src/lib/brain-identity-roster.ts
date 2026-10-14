import { HostedHermesStatusError, readHostedHermesJson } from "./hosted-hermes-status";

export type BrainIdentity = {
  npub: string; role: string; kind: "human" | "agent" | null;
  name: string | null; email: string | null; ownerEmail: string | null;
  folders: { id: string; state: string }[];
};

function invalid(): never {
  throw new HostedHermesStatusError("This agent needs a compatible page update. Try again after it updates.", "unsupported");
}
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
  return value as Record<string, unknown>;
}
function text(value: unknown, max = 512): string {
  if (typeof value !== "string" || !value.trim() || value.length > max) return invalid();
  return value;
}
function optional(value: unknown) { return value == null ? null : text(value); }
function list(value: unknown, max: number) {
  if (!Array.isArray(value) || value.length > max) return invalid();
  return value.map(record);
}

export function parseBrainIdentities(value: unknown, brainId: string): BrainIdentity[] {
  const report = record(value);
  if (report.version !== 1 || report.brainId !== brainId) return invalid();
  const identities = list(report.identities, 1024).map((row): BrainIdentity => {
    const npub = text(row.npub, 128);
    if (!/^npub1[023456789acdefghjklmnpqrstuvwxyz]{58}$/.test(npub)) return invalid();
    const kind = optional(row.kind);
    if (kind !== null && kind !== "human" && kind !== "agent") return invalid();
    return { npub, role: text(row.role, 64), kind, name: optional(row.name), email: optional(row.email),
      ownerEmail: optional(row.ownerEmail),
      folders: list(row.folders, 500).map(folder => ({ id: text(folder.id, 256), state: text(folder.state, 64) })) };
  });
  if (new Set(identities.map(row => row.npub)).size !== identities.length) return invalid();
  return identities;
}

export async function readBrainIdentities(runtimeId: string, brainId: string, signal: AbortSignal) {
  if (!/^[A-Za-z0-9_][A-Za-z0-9_-]{0,127}$/.test(brainId)) return invalid();
  return parseBrainIdentities(await readHostedHermesJson(runtimeId,
    `api/plugins/finite-brain/identities/${brainId}`, signal), brainId);
}
