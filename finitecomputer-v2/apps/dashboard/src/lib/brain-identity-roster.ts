import { HostedHermesStatusError, readHostedHermesJson } from "./hosted-hermes-status";
import { parseRosterChanges, type RosterChanges } from "./brain-roster-changes";

export type BrainOperatorAnnotation = {
  kind: "historical_identity" | "invitation_key"; label: string; observedAt: string;
};
export type BrainIdentity = {
  npub: string; role: string; kind: "human" | "agent" | null;
  name: string | null; email: string | null; ownerEmail: string | null;
  /** The public NIP-05 name Brain stored for this exact key, shown only when Core resolved nothing. */
  publicName: string | null; publicNameStoredAt: string | null;
  lifecycle: "active" | "offboarding" | "retired" | null;
  operatorAnnotation: BrainOperatorAnnotation | null;
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
const rfc3339 = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,9})?(?:Z|[+-](\d{2}):(\d{2}))$/;
/** Like the plugin's datetime.fromisoformat: a real calendar day from year 1, hour 0-23, minute and second 0-59 (no leap second), offset hour 0-23 and minute 0-59. */
function time(value: unknown) {
  if (value == null) return null;
  const stamp = text(value, 64);
  const parts = rfc3339.exec(stamp);
  if (!parts) return invalid();
  const [year, month, day, hour, minute, second, offsetHour, offsetMinute] = parts.slice(1).map(part => Number(part ?? 0));
  // Date rolls 02-30 into March and 24:00 into the next day; a rolled component no longer matches.
  const date = new Date(0);
  date.setUTCFullYear(year, month - 1, day);
  date.setUTCHours(hour, minute, second);
  const real = year >= 1 && date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day
    && date.getUTCHours() === hour && date.getUTCMinutes() === minute && date.getUTCSeconds() === second;
  return real && offsetHour <= 23 && offsetMinute <= 59 ? stamp : invalid();
}
function list(value: unknown, max: number) {
  if (!Array.isArray(value) || value.length > max) return invalid();
  return value.map(record);
}


/** Optional attribution is withheld on malformed input; it never becomes a name or contact. */
function annotation(value: unknown): BrainOperatorAnnotation | null {
  if (value == null) return null;
  try {
    const item = record(value);
    if (Object.keys(item).sort().join(",") !== "kind,label,observedAt"
      || (item.kind !== "historical_identity" && item.kind !== "invitation_key")) return null;
    const label = text(item.label, 120);
    if (label.trim() !== label || !/^[\x20-\x7e]+$/.test(label)) return null;
    const observedAt = time(item.observedAt);
    if (!observedAt || observedAt.length > 40 || !observedAt.endsWith("Z")) return null;
    return { kind: item.kind, label, observedAt };
  } catch { return null; }
}

export function parseBrainIdentities(value: unknown, brainId: string): BrainIdentity[] {
  const report = record(value);
  if (report.version !== 1 || report.brainId !== brainId) return invalid();
  const identities = list(report.identities, 1024).map((row): BrainIdentity => {
    const npub = text(row.npub, 128);
    if (!/^npub1[023456789acdefghjklmnpqrstuvwxyz]{58}$/.test(npub)) return invalid();
    const kind = optional(row.kind);
    if (kind !== null && kind !== "human" && kind !== "agent") return invalid();
    // Older plugins omit both fields; a stored time without a name is contradictory.
    const publicName = optional(row.publicName), publicNameStoredAt = time(row.publicNameStoredAt);
    if (publicName === null && publicNameStoredAt !== null) return invalid();
    const lifecycle = row.lifecycle ?? null;
    if (lifecycle !== null && (kind !== "agent" || (lifecycle !== "active" && lifecycle !== "offboarding" && lifecycle !== "retired"))) return invalid();
    const operatorAnnotation = kind === null && row.name == null && row.email == null && row.ownerEmail == null
      ? annotation(row.operatorAnnotation) : null;
    return { npub, role: text(row.role, 64), kind, name: optional(row.name), email: optional(row.email),
      ownerEmail: kind === "agent" ? optional(row.ownerEmail) : null, publicName, publicNameStoredAt,
      lifecycle, operatorAnnotation,
      folders: list(row.folders, 500).map(folder => ({ id: text(folder.id, 256), state: text(folder.state, 64) })) };
  });
  if (new Set(identities.map(row => row.npub)).size !== identities.length) return invalid();
  return identities;
}

export type BrainRoster = { identities: BrainIdentity[]; changes: RosterChanges | null };

export async function readBrainIdentities(runtimeId: string, brainId: string, signal: AbortSignal): Promise<BrainRoster> {
  if (!/^[A-Za-z0-9_][A-Za-z0-9_-]{0,127}$/.test(brainId)) return invalid();
  const report = await readHostedHermesJson(runtimeId, `api/plugins/finite-brain/identities/${brainId}`, signal);
  return { identities: parseBrainIdentities(report, brainId), changes: parseRosterChanges(report) };
}
