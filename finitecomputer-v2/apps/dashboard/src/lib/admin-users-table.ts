import { finitePrivateProfileLabel, type RuntimeFinitePrivateGrantSummary } from "./admin-ops";
import type { CoreAdminRuntimeOverview, CoreFinitePrivateAdminAccount, CoreFinitePrivateLimitProfile, CoreFinitePrivateAdminState } from "./core-client";

/** Display and search share the same values, including formatted numbers. */
export function adminUserTableCells(
  runtime: CoreAdminRuntimeOverview,
  account: CoreFinitePrivateAdminAccount | null,
  grant: RuntimeFinitePrivateGrantSummary | null,
  profiles: CoreFinitePrivateLimitProfile[] = [],
  state?: CoreFinitePrivateAdminState | null,
): { label: string; value: string; numeric?: boolean; usage?: { usedUnits: number; limitUnits: number } }[] {
  const profileId = grant?.limitProfileId ?? account?.grant.limit_profile_id;
  const profile = profiles.find((candidate) => candidate.id === profileId);
  const grantId = grant?.grantId ?? account?.grant.id;
  const fullGrant = state?.grants.find((candidate) => candidate.id === grantId)
    ?? (account?.grant.id === grantId ? account?.grant : undefined);
  const key = state?.apiKeys.find((candidate) => candidate.id === grant?.keyId)
    ?? account?.apiKeys.find((candidate) => candidate.id === grant?.keyId);
  const units = grant?.currentWindowUsedUnits ?? account?.grant.current_window_used_units;
  const number = (value: number | null | undefined) => value == null ? "Unavailable" : new Intl.NumberFormat("en-US").format(value);
  const cell = (label: string, value: string | null | undefined) => ({ label, value: value || "Unavailable" });
  return [
    cell("Email", runtime.owner_email ?? account?.email),
    cell("Name", null), // The admin overview does not expose the human's name.
    cell("Agent name", runtime.project_display_name),
    cell("Status", runtime.runtime_status),
    {
      label: "Usage (weighted tokens)",
      value: units != null && profile && profile.burst_limit_units > 0
        ? `${number(units)} of ${number(profile.burst_limit_units)} weighted tokens ${Math.max(0, Math.min(100, Math.round(units / profile.burst_limit_units * 100)))}% used`
        : number(units),
      numeric: true,
      usage: units != null && profile && profile.burst_limit_units > 0
        ? { usedUnits: units, limitUnits: profile.burst_limit_units }
        : undefined,
    },
    { label: "Burst limit", value: number(profile?.burst_limit_units), numeric: true },
    cell("Limit profile", profileId ? finitePrivateProfileLabel(profileId) : null),
    cell("Grant status", grant?.grantStatus ?? account?.grant.status),
    cell("Window started", grant?.currentWindowStartedAt ?? account?.grant.current_window_started_at),
    cell("Health", runtime.runtime_health?.status),
    cell("Health reason", runtime.runtime_health?.reason),
    cell("Lifecycle", runtime.lifecycle_status),
    cell("Hermes", runtime.hermes_available == null ? "Unknown" : runtime.hermes_available ? "Yes" : "No"),
    cell("Host", runtime.source_host_id),
    cell("Machine", runtime.source_machine_id),
    cell("Artifact version", runtime.runtime_artifact_version_label),
    cell("Artifact ID", runtime.runtime_artifact_id),
    cell("Runtime ID", runtime.agent_runtime_id),
    cell("Project ID", runtime.project_id),
    cell("User ID", grant?.grantUserId ?? account?.userId),
    cell("Grant ID", grant?.grantId ?? account?.grant.id),
    cell("Key ID", grant?.keyId),
    cell("Key status", grant?.keyStatus),
    cell("Grant match", grant?.matchScope),
    { label: "Active keys", value: number(runtime.active_finite_private_key_count), numeric: true },
    cell("Runtime link", runtime.runtime_link_active ? "Linked" : "Unlinked"),
    cell("Last heartbeat", runtime.last_heartbeat_at),
    cell("Status updated", runtime.status_updated_at),
    cell("Runtime updated", runtime.runtime_updated_at),
    cell("Health reported", runtime.runtime_health?.reported_at),
    cell("Health observed", runtime.runtime_health?.observed_at),
    cell("Agent public key", runtime.runtime_health?.agent_npub),
    cell("Offboarding phase", runtime.offboarding_phase),
    cell("Supported operations", runtime.runtime_capabilities
      ? Object.entries(runtime.runtime_capabilities).map(([name, enabled]) => `${name}: ${enabled ? "yes" : "no"}`).join(" · ") || "None"
      : "Unavailable"),
    cell("Health cadence (seconds)", number(runtime.runtime_health?.report_interval_seconds)),
    cell("Profile ID", profileId),
    cell("Burst window (seconds)", number(profile?.burst_window_seconds)),
    cell("Weekly limit", profile ? profile.weekly_limit_units == null ? "Not set" : number(profile.weekly_limit_units) : null),
    cell("Profile created", profile?.created_at),
    cell("Profile updated", profile?.updated_at),
    cell("Grant created", fullGrant?.created_at),
    cell("Grant updated", fullGrant?.updated_at),
    cell("Burst window epoch", number(fullGrant?.burst_window_epoch)),
    cell("Key project ID", key?.project_id ?? grant?.keyProjectId),
    cell("Key runtime ID", key?.agent_runtime_id ?? grant?.keyAgentRuntimeId),
    cell("Key created", key?.created_at),
    cell("Key updated", key?.updated_at),
    cell("Account email", account?.email),
    cell("Account user ID", account?.userId),
    cell("Account projects", account?.projects.map((project) => `${project.displayName} · ${project.id} · ${project.agentRuntimeId ?? "No runtime"}`).join("; ")),
    cell("Account keys", account?.apiKeys.map((apiKey) => `${apiKey.id} · ${apiKey.status} · grant ${apiKey.grant_id} · project ${apiKey.project_id ?? "none"} · runtime ${apiKey.agent_runtime_id ?? "none"} · created ${apiKey.created_at} · updated ${apiKey.updated_at}`).join("; ")),
    cell("Published URLs", runtime.published_app_urls.join(" · ")),
  ];
}

/** Correlate only explicit grant/key/target identifiers, never email or sort order. */
export function adminUserAuditEvents(
  runtime: CoreAdminRuntimeOverview,
  grant: RuntimeFinitePrivateGrantSummary | null,
  state?: CoreFinitePrivateAdminState | null,
) {
  const keys = state?.apiKeys.filter((key) => key.agent_runtime_id === runtime.agent_runtime_id || key.project_id === runtime.project_id) ?? [];
  const keyIds = new Set(keys.map((key) => key.id));
  const grantIds = new Set(keys.map((key) => key.grant_id));
  if (grant) grantIds.add(grant.grantId);
  const targets = new Set([runtime.agent_runtime_id, runtime.project_id, ...keyIds, ...grantIds]);
  return (state?.adminAuditEvents ?? []).filter((event) =>
    targets.has(event.target_id) || (event.grant_id != null && grantIds.has(event.grant_id)) || (event.api_key_id != null && keyIds.has(event.api_key_id))
  ).sort((a, b) => b.created_at.localeCompare(a.created_at));
}

export type AdminTableSort = { column: string; direction: "ascending" | "descending" };
const tableCollator = new Intl.Collator("en", { numeric: true, sensitivity: "base" });

export function compareAdminTableCells(
  left: { value: string; usage?: { usedUnits: number } } | undefined,
  right: { value: string; usage?: { usedUnits: number } } | undefined,
  direction: AdminTableSort["direction"],
) {
  const value = (cell: typeof left): string | number | null => {
    if (!cell || ["Unavailable", "Unknown", "Not set"].includes(cell.value)) return null;
    if (cell.usage) return cell.usage.usedUnits;
    if (/^\d{4}-\d{2}-\d{2}T/.test(cell.value)) {
      const timestamp = Date.parse(cell.value);
      if (Number.isFinite(timestamp)) return timestamp;
    }
    if (/^-?[\d,]+(?:\.\d+)?$/.test(cell.value)) return Number(cell.value.replaceAll(",", ""));
    return cell.value;
  };
  const a = value(left);
  const b = value(right);
  // Missing values stay last in both directions; ties preserve the input order.
  if (a == null) return b == null ? 0 : 1;
  if (b == null) return -1;
  const comparison = typeof a === "number" && typeof b === "number" ? a - b : tableCollator.compare(String(a), String(b));
  if (comparison === 0) return 0;
  return direction === "ascending" ? comparison : -comparison;
}
