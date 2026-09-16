import { AgentOnboardingStageSync } from "@/components/agent-onboarding-progress";
import { StatusPrism } from "@/components/status-prism";
import { Button } from "@/components/ui/button";

export function CoreAgentReadyPanel({
  chatHref,
  name,
}: {
  chatHref: string;
  name: string;
}) {
  return (
    <section
      className="grid w-full content-center justify-items-center gap-6 text-center"
      aria-labelledby="agent-ready-title"
    >
      <AgentOnboardingStageSync stage="ready" />
      <StatusPrism state="happy" className="cursor-default" />
      <div className="grid gap-2">
        <h1
          id="agent-ready-title"
          className="font-sans text-3xl leading-tight font-medium tracking-[-0.02em] sm:text-5xl"
        >
          {name} is alive!
        </h1>
        <p className="type-body-lg text-muted-foreground">
          Go introduce yourself to {name}.
        </p>
      </div>
      <Button asChild size="xl">
        <a href={chatHref}>Continue to chat</a>
      </Button>
    </section>
  );
}
