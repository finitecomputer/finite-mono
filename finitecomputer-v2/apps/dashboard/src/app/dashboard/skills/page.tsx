import headingStyles from "@/styles/agent-page-heading.module.css";
import { Layers3Icon } from "lucide-react";

import { SkillsCatalogBrowser } from "@/components/skills-catalog-browser";
import { loadBaselineSkillsCatalog } from "@/lib/skills-catalog";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

export const dynamic = "force-dynamic";

export default async function SkillsDashboardPage({ searchParams }: {
  searchParams: Promise<{ machine?: string | string[] }>;
}) {
  const { machine } = await searchParams;
  const machineId = typeof machine === "string" ? machine : undefined;
  const access = machineId ? await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" }) : null;
  const model = await loadBaselineSkillsCatalog().catch((error) => {
    console.error("[skills] failed to load skills catalog", error);
    return null;
  });

  return (
    <div className={`ocean-page-stack ${headingStyles.page}`}>
      {!model && <header className={`${headingStyles.stack} mb-2`}>
        <h1 className={headingStyles.title}>Skills</h1>
        <p className={headingStyles.subtitle}>Skills are currently unavailable.</p>
      </header>}

      {model ? (
        <SkillsCatalogBrowser
          skills={model.skills}
          summary={`${model.totalSkillCount} ${model.totalSkillCount === 1 ? "skill" : "skills"} available to ${access?.displayName ?? "your agent"}.`}
        />
      ) : (
        <section className="ocean-utility-card">
          <div className="ocean-utility-card__header">
            <span className="ocean-utility-card__icon" aria-hidden>
              <Layers3Icon className="size-5" />
            </span>
            <div>
              <h2 className="ocean-utility-card__title">Skill catalog unavailable</h2>
              <p className="text-sm text-muted-foreground">
                The runtime still syncs skills from GitHub, but the dashboard catalog could not be loaded.
              </p>
            </div>
          </div>
        </section>
      )}
    </div>
  );
}
