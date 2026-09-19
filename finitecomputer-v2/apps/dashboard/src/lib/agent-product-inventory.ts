import { HostedHermesStatusError, readHostedHermesJson } from "./hosted-hermes-status";
import type { SiteListItem } from "../components/sites-browser";

export type BrainRow = {
  id: string; name: string; kind: "Personal" | "Organization"; role: string;
  folders: { id: string; name: string }[] | null; pending: boolean;
};
export type BrainInventory = { brains: BrainRow[] };
export type SitesInventory = { sites: SiteListItem[]; sourceOnlyProjects: number };
const roles: Record<string, string> = {
  owner: "Owner", personal_agent: "Personal agent", admin: "Admin",
  member: "Member", guest: "Guest", invited: "Pending",
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
function rows(value: unknown, max: number): Record<string, unknown>[] {
  if (!Array.isArray(value) || value.length > max) return invalid();
  return value.map(record);
}
function httpsUrl(value: unknown): string {
  const source = text(value, 2048);
  let url: URL;
  try { url = new URL(source); } catch { return invalid(); }
  if (url.protocol !== "https:" || url.username || url.password) return invalid();
  return source;
}
function uniqueIds<T extends { id: string }>(items: T[]): T[] {
  if (new Set(items.map(item => item.id)).size !== items.length) return invalid();
  return items;
}
export function parseBrainInventory(value: unknown): BrainInventory {
  const data = record(value);
  if (data.version !== 1) return invalid();
  const brains = rows(data.brains, 100).map((brain): BrainRow => {
    if (!(brain.kind === "personal" || brain.kind === "organization")) return invalid();
    const role = text(brain.role, 64);
    if (!Object.hasOwn(roles, role)) return invalid();
    const folders = brain.folders === null ? null : uniqueIds(rows(brain.folders, 500).map(folder => ({
      id: text(folder.id, 256), name: text(folder.name),
    })));
    if (role === "invited" && folders !== null) return invalid();
    return { id: text(brain.id, 256), name: text(brain.name),
      kind: brain.kind === "personal" ? "Personal" : "Organization",
      role: roles[role], pending: role === "invited", folders };
  });
  return { brains: uniqueIds(brains).sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id)) };
}
export function parseSitesInventory(value: unknown): SitesInventory {
  const data = record(value);
  if (data.version !== 1 || !Number.isSafeInteger(data.sourceOnlyProjects) ||
      Number(data.sourceOnlyProjects) < 0 || Number(data.sourceOnlyProjects) > 500) return invalid();
  const sites = rows(data.sites, 500).map((site): SiteListItem => {
    if (!["private", "shared", "public"].includes(String(site.visibility)) ||
        typeof site.canEdit !== "boolean" || typeof site.published !== "boolean") return invalid();
    const access = site.visibility as SiteListItem["access"];
    const status = text(site.status, 64);
    return {
      id: text(site.id, 256), title: text(site.name), url: httpsUrl(site.url), access,
      accessLabel: access === "private" ? "Private" : access === "shared" ? "Shared" : "Public",
      canEdit: site.canEdit, repositoryUrl: httpsUrl(site.repositoryUrl),
      // An allocated address is not necessarily a published website.
      statusLabel: !site.published ? "Not published" : status === "published" ? undefined : status.replaceAll("_", " "),
      published: site.published && status === "published",
    };
  });
  if (sites.length + Number(data.sourceOnlyProjects) > 500) return invalid();
  return { sites: uniqueIds(sites).sort((a, b) => a.title.localeCompare(b.title) || a.id.localeCompare(b.id)),
    sourceOnlyProjects: data.sourceOnlyProjects as number };
}
export async function readBrainInventory(runtimeId: string, signal: AbortSignal) {
  return parseBrainInventory(await readHostedHermesJson(runtimeId, "api/plugins/finite-brain/overview", signal));
}
export async function readSitesInventory(runtimeId: string, signal: AbortSignal) {
  return parseSitesInventory(await readHostedHermesJson(runtimeId, "api/plugins/finite-sites/overview", signal));
}
