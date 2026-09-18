"use client";

import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { RefreshCwIcon, SearchIcon, XIcon } from "lucide-react";
import Link from "next/link";

import { groupAgentSkills, readAgentSkills, skillCategoryLabel, type AgentSkill } from "@/lib/agent-skills";
import { HostedHermesStatusError } from "@/lib/hosted-hermes-status";
import headingStyles from "@/styles/agent-page-heading.module.css";
import styles from "@/styles/skills-catalog.module.css";

const SkillCard = memo(function SkillCard({ skill }: { skill: AgentSkill }) {
  return (
    <article className="ocean-skill-card">
      <div className="ocean-skill-card__meta">
        <span className="ocean-chip">{skillCategoryLabel(skill.category)}</span>
        {!skill.enabled && <span className="ocean-chip ocean-chip--muted">Disabled</span>}
      </div>
      <div className="ocean-skill-card__copy"><h3>{skill.name}</h3><p>{skill.description}</p></div>
    </article>
  );
});

/** The server keys this component by account + agent, so a scope change clears
 * private results before paint. Requests are operation-local and cancel on exit. */
export function AgentSkillsBrowser({ runtimeId, agentName }: { runtimeId: string; agentName: string }) {
  const [skills, setSkills] = useState<AgentSkill[] | null>(null);
  const [loadedAt, setLoadedAt] = useState<Date | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [accessUnavailable, setAccessUnavailable] = useState(false);
  const [query, setQuery] = useState("");
  const pending = useRef<AbortController | null>(null);

  const refresh = useCallback(async () => {
    pending.current?.abort();
    const controller = new AbortController();
    pending.current = controller;
    setBusy(true);
    setError(null);
    setAccessUnavailable(false);
    try {
      const result = await readAgentSkills(runtimeId, controller.signal);
      if (controller.signal.aborted) return;
      setSkills(result);
      setLoadedAt(new Date());
    } catch (caught) {
      if (controller.signal.aborted) return;
      if (caught instanceof HostedHermesStatusError && caught.kind !== "request") {
        setSkills(null);
        setLoadedAt(null);
      }
      setAccessUnavailable(caught instanceof HostedHermesStatusError && caught.kind === "access");
      setError(caught instanceof HostedHermesStatusError ? caught.message : "Skills could not be loaded. Try again.");
    } finally {
      if (!controller.signal.aborted) setBusy(false);
    }
  }, [runtimeId]);

  useEffect(() => {
    void refresh();
    return () => pending.current?.abort();
  }, [refresh]);

  const groups = useMemo(() => groupAgentSkills(skills ?? [], query), [skills, query]);
  const summary = skills === null
    ? busy ? `Loading skills for ${agentName}…` : `Skills are unavailable for ${agentName}.`
    : `${skills.length} ${skills.length === 1 ? "skill" : "skills"} discovered for ${agentName}.`;

  return (
    <section className={styles.catalog} aria-busy={busy}>
      <header className={headingStyles.stack}>
        <h1 className={headingStyles.title}>Skills</h1>
        <div className={styles.toolbar}>
          <p className={headingStyles.subtitle} aria-live="polite">{summary}</p>
          <div className={styles.search}>
            <SearchIcon aria-hidden="true" size={16} />
            <input aria-label="Search skills" placeholder="Search skills" value={query} onChange={(event) => setQuery(event.target.value)} type="search" />
            <button type="button" aria-label="Clear search" style={{ visibility: query ? "visible" : "hidden" }} onClick={() => setQuery("")}><XIcon size={16} /></button>
          </div>
        </div>
      </header>

      {error && <p role="alert" className="text-sm text-muted-foreground">{skills !== null ? "Showing the last successful list; it may be out of date. " : ""}{error}</p>}
      {accessUnavailable && <Link className="text-sm underline underline-offset-4" href={`/dashboard/machines/${encodeURIComponent(runtimeId)}/connections#web-access`}>Manage agent web access</Link>}
      {skills !== null && (skills.length === 0
        ? <div className="ocean-empty-state">No skills were discovered for {agentName}.</div>
        : groups.length === 0
          ? <div className="ocean-empty-state">No skills match <span className="font-medium text-foreground">{query}</span>.</div>
          : groups.map(({ category, skills: entries }) => (
            <section key={category} aria-label={skillCategoryLabel(category)} className="space-y-3">
              <h2 className="text-sm font-medium">{skillCategoryLabel(category)} <span className="text-muted-foreground">({entries.length})</span></h2>
              <div className="ocean-skill-grid">{entries.map((skill) => <SkillCard key={skill.name} skill={skill} />)}</div>
            </section>
          )))}
      <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
        <button type="button" disabled={busy} onClick={() => void refresh()} className="inline-flex min-h-9 items-center gap-2 rounded-md px-2 focus-visible:outline-2 focus-visible:outline-offset-2 disabled:opacity-50">
          <RefreshCwIcon size={14} aria-hidden="true" />{busy ? "Loading…" : "Refresh"}
        </button>
        {loadedAt && <span>Last loaded <time dateTime={loadedAt.toISOString()}>{loadedAt.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}</time></span>}
      </div>
    </section>
  );
}
