import { HostedHermesStatusError, readHostedHermesJson } from "./hosted-hermes-status";

export type AgentSkill = { name: string; description: string; category: string; enabled: boolean };

/** Project only display metadata. Native paths, content, usage and provenance
 * are not part of this page's contract. Reject malformed/partial inventories. */
export function parseAgentSkills(value: unknown): AgentSkill[] {
  const invalid = () => new HostedHermesStatusError("The agent returned an unexpected skills list. Try refreshing.");
  if (!Array.isArray(value) || value.length > 4000) throw invalid();
  const names = new Set<string>();
  return value.map((entry: unknown) => {
    if (!entry || typeof entry !== "object") throw invalid();
    const skill = entry as Record<string, unknown>;
    if (
      typeof skill.name !== "string" || !skill.name.trim() || skill.name.length > 256 ||
      names.has(skill.name) || typeof skill.description !== "string" || skill.description.length > 4096 ||
      typeof skill.enabled !== "boolean" ||
      !(skill.category == null || (typeof skill.category === "string" && skill.category.length <= 256))
    ) throw invalid();
    names.add(skill.name);
    return {
      name: skill.name, description: skill.description, enabled: skill.enabled,
      category: typeof skill.category === "string" && skill.category.trim() ? skill.category : "general",
    };
  });
}

export function skillCategoryLabel(category: string) {
  return category.replace(/[-_/]+/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

export function groupAgentSkills(skills: readonly AgentSkill[], query: string) {
  const normalized = query.trim().toLocaleLowerCase();
  const groups = new Map<string, AgentSkill[]>();
  for (const skill of skills) {
    if (normalized && ![skill.name, skill.description, skill.category, skillCategoryLabel(skill.category)]
      .some((field) => field.toLocaleLowerCase().includes(normalized))) continue;
    const group = groups.get(skill.category) ?? [];
    group.push(skill);
    groups.set(skill.category, group);
  }
  return [...groups].sort(([a], [b]) => skillCategoryLabel(a).localeCompare(skillCategoryLabel(b)))
    .map(([category, entries]) => ({ category, skills: entries.sort((a, b) => a.name.localeCompare(b.name)) }));
}

export async function readAgentSkills(runtimeId: string, signal: AbortSignal) {
  return parseAgentSkills(await readHostedHermesJson(runtimeId, "api/skills", signal));
}
