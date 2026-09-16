"use client";

import { memo, useMemo, useState } from "react";

import { SearchIcon, XIcon } from "lucide-react";

import headingStyles from "@/styles/agent-page-heading.module.css";
import styles from "@/styles/skills-catalog.module.css";
import type { BaselineSkillCatalogEntry } from "@/lib/skills-catalog";

const SkillCard = memo(function SkillCard({ skill }: { skill: BaselineSkillCatalogEntry }) {
  return (
    <article className="ocean-skill-card">
      <div className="ocean-skill-card__meta">
        <span className="ocean-chip">{skill.category}</span>
        {skill.version ? <span className="ocean-chip">v{skill.version}</span> : null}
      </div>
      <div className="ocean-skill-card__copy">
        <h3>{skill.name}</h3>
        <p>{skill.description}</p>
      </div>
      {skill.setupLabels.length > 0 ? (
        <div className="ocean-skill-card__setup">
          {skill.setupLabels.map((label) => (
            <span key={label} className="ocean-chip ocean-chip--muted">
              {label}
            </span>
          ))}
        </div>
      ) : null}
    </article>
  );
});

export function SkillsCatalogBrowser({ skills, summary }: { skills: BaselineSkillCatalogEntry[]; summary: string }) {
  const [query, setQuery] = useState("");

  const filteredSkills = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) {
      return skills;
    }
    return skills.filter((skill) =>
      [skill.name, skill.description, skill.category, ...skill.setupLabels]
        .join(" ")
        .toLowerCase()
        .includes(normalized)
    );
  }, [query, skills]);

  return (
    <section className={styles.catalog}>
      <header className={headingStyles.stack}>
        <h1 className={headingStyles.title}>Skills</h1>
        <div className={styles.toolbar}>
          <p className={headingStyles.subtitle}>{summary}</p>
          <div className={styles.search}>
            <SearchIcon aria-hidden="true" size={16} />
            <input aria-label="Search skills" placeholder="Search skills" value={query} onChange={(event) => setQuery(event.target.value)} type="search" />
            <button type="button" aria-label="Clear search" style={{ visibility: query ? "visible" : "hidden" }} onClick={() => setQuery("")}><XIcon size={16} /></button>
          </div>
        </div>
      </header>

      {filteredSkills.length > 0 ? (
        <div className="ocean-skill-grid">
          {filteredSkills.map((skill) => (
            <SkillCard key={skill.managedRelpath} skill={skill} />
          ))}
        </div>
      ) : (
        <div className="ocean-empty-state">
          No skills match <span className="font-medium text-foreground">{query}</span>.
        </div>
      )}
    </section>
  );
}
