import {
  ExternalLinkIcon,
  Layers3Icon,
} from "lucide-react";
import { redirect } from "next/navigation";

import { SkillsCatalogBrowser } from "@/components/skills-catalog-browser";
import { loadCoreMe } from "@/lib/core-client";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";
import { loadBaselineSkillsCatalog } from "@/lib/skills-catalog";

export const dynamic = "force-dynamic";

export default async function SkillsDashboardPage() {
  const [viewer, core] = await Promise.all([
    loadOptionalViewerContext(),
    loadCoreMe({ cacheMode: "swr" }),
  ]);
  if (core.configured && !viewer.isAdmin) {
    redirect("/dashboard");
  }

  const model = await loadBaselineSkillsCatalog().catch((error) => {
    console.error("[skills] failed to load skills catalog", error);
    return null;
  });

  return (
    <div className="ocean-page-stack">
      <section className="ocean-page-hero">
        <div className="ocean-page-hero__main">
          <span className="ocean-page-hero__icon">
            <Layers3Icon className="size-5" />
          </span>
          <div>
            <h1 className="ocean-page-hero__title">Skills</h1>
            <p className="ocean-page-hero__description">Shared baseline skills available to every machine.</p>
          </div>
        </div>

        <div className="ocean-metric-grid">
          <div className="ocean-metric">
            <span>{model?.totalSkillCount ?? "..."}</span>
            <small>Skills</small>
          </div>
          <div className="ocean-metric">
            <span>{model?.categoryCount ?? "..."}</span>
            <small>Categories</small>
          </div>
          <a
            href="https://hermes-agent.nousresearch.com/docs/user-guide/features/skills/"
            target="_blank"
            rel="noreferrer"
            className="ocean-metric ocean-metric--link"
          >
            <span>Docs</span>
            <small>
              Hermes
              <ExternalLinkIcon className="size-3.5" />
            </small>
          </a>
        </div>
      </section>

      {model ? (
        <SkillsCatalogBrowser skills={model.skills} />
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
