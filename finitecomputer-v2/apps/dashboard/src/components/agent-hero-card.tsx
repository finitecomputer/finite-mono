import type { ReactNode } from "react";

import { StatusPrism } from "@/components/status-prism";

export function AgentHeroCard({
  actions,
  description,
  name,
  state,
}: {
  actions: ReactNode;
  description: string;
  name: string;
  state: "happy" | "working" | "stuck";
}) {
  return (
    <section className="ocean-status-card" data-cube-state={state}>
      <div className="ocean-status-card__inner">
        <StatusPrism state={state} className="justify-self-center" />
        <div className="ocean-status-card__copy">
          <h2 className="ocean-status-card__title">{name}</h2>
          <p className="ocean-status-card__description">{description}</p>
          <div className="ocean-status-card__actions">{actions}</div>
        </div>
      </div>
    </section>
  );
}
