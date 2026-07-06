"use client";

import { memo, useMemo, useState } from "react";

import { SearchIcon } from "lucide-react";

import { Input } from "@/components/ui/input";
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

export function SkillsCatalogBrowser({ skills }: { skills: BaselineSkillCatalogEntry[] }) {
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
    <section className="ocean-catalog-section">
      <div className="ocean-catalog-toolbar">
        <div className="ocean-catalog-toolbar__copy">
          <h2>Catalog</h2>
          <p>{filteredSkills.length} of {skills.length} skills</p>
        </div>
        <div className="ocean-catalog-search">
          <SearchIcon className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search skills"
            className="h-10 rounded-lg pl-10"
          />
        </div>
      </div>

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
