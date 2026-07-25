"use client";

import { useEffect, useRef, useState } from "react";
import { CheckIcon, CreditCardIcon, KeyRoundIcon } from "lucide-react";

import { FiniteBrand } from "@/components/finite-brand";
import { FiniteLoader } from "@/components/finite-loader";
import { StatusPrism } from "@/components/status-prism";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import "@/styles/ocean-shell.css";

const STAGES = ["name", "preflight", "payment", "ready"] as const;
const PREFLIGHT_STEPS = [
  "Creating private workspace",
  "Spinning up the box",
  "Loading core skills",
  "Securing memory",
  "Opening a private channel",
] as const;
const SIMULATED_PREFLIGHT_STEP_MS = 2550;
const HEADLINE_CLASS =
  "font-sans text-3xl leading-[1.08] font-medium tracking-[-0.02em] text-balance sm:text-5xl";

type Stage = (typeof STAGES)[number];

export function OnboardingFlowPreview() {
  const [stage, setStage] = useState<Stage>("name");
  const [draftName, setDraftName] = useState("Scout");
  const [agentName, setAgentName] = useState("Scout");
  const [preflightIndex, setPreflightIndex] = useState(-1);
  const [showLaunchCode, setShowLaunchCode] = useState(false);
  const timersRef = useRef<number[]>([]);

  const stageIndex = STAGES.indexOf(stage);

  useEffect(() => {
    if (stage !== "preflight") return;

    timersRef.current = PREFLIGHT_STEPS.map((_, index) =>
      window.setTimeout(
        () => setPreflightIndex(index),
        index * SIMULATED_PREFLIGHT_STEP_MS
      )
    );
    timersRef.current.push(
      window.setTimeout(
        () => setStage("payment"),
        (PREFLIGHT_STEPS.length + 0.65) * SIMULATED_PREFLIGHT_STEP_MS
      )
    );

    return () => {
      timersRef.current.forEach(window.clearTimeout);
      timersRef.current = [];
    };
  }, [stage]);

  function beginPreflight() {
    const nextName = draftName.trim() || "Scout";
    setAgentName(nextName);
    setPreflightIndex(-1);
    setStage("preflight");
  }

  return (
    <div className="ocean-shell relative min-h-screen overflow-hidden bg-background text-foreground">
      <div
        className="pointer-events-none absolute inset-0 opacity-70"
        aria-hidden
        style={{
          background:
            "radial-gradient(circle at 50% 32%, color-mix(in srgb, var(--accent-blue) 12%, transparent), transparent 38%)",
        }}
      />

      <div className="relative z-10 grid min-h-screen grid-rows-[auto_1fr] px-5 py-5 sm:px-8 sm:py-7">
        <header className="flex items-center justify-between gap-4">
          <FiniteBrand />
          <ol
            className="grid w-28 grid-cols-4 gap-1"
            aria-label="Onboarding progress"
          >
            {STAGES.map((item, index) => (
              <li
                key={item}
                className={`h-0.5 ${index <= stageIndex ? "bg-foreground" : "bg-border"}`}
                aria-current={index === stageIndex ? "step" : undefined}
              >
                <span className="sr-only">
                  {item}: {index < stageIndex ? "complete" : index === stageIndex ? "current" : "upcoming"}
                </span>
              </li>
            ))}
          </ol>
        </header>

        <main className="grid place-items-center py-8">
          <div className="grid min-h-[34rem] w-full grid-rows-[8rem_1fr] justify-items-center">
            <StageVisual stage={stage} agentName={agentName} />

            <div className="grid w-full content-start justify-items-center pt-6">
              {stage === "name" ? (
                <section
                  className="grid w-full max-w-3xl justify-items-center gap-8 text-center"
                  aria-labelledby="onboarding-name-title"
                >
                  <h1
                    id="onboarding-name-title"
                    className={`max-w-2xl ${HEADLINE_CLASS}`}
                  >
                    Give your agent a name.
                  </h1>
                  <form
                    className="grid w-full max-w-sm grid-cols-1 items-stretch gap-2 sm:grid-cols-[1fr_auto]"
                    onSubmit={(event) => {
                      event.preventDefault();
                      beginPreflight();
                    }}
                  >
                    <Label htmlFor="preview-agent-name" className="sr-only">
                      Agent name
                    </Label>
                    <Input
                      id="preview-agent-name"
                      value={draftName}
                      onChange={(event) => setDraftName(event.target.value)}
                      maxLength={80}
                      autoFocus
                      className="h-12"
                    />
                    <Button type="submit" size="xl" disabled={!draftName.trim()}>
                      Continue
                    </Button>
                  </form>
                </section>
              ) : null}

              {stage === "preflight" ? (
                <section
                  className="grid w-full max-w-xl justify-items-center gap-7 text-center"
                  aria-labelledby="onboarding-preflight-title"
                >
                  <h1 id="onboarding-preflight-title" className={HEADLINE_CLASS}>
                    Preparing {agentName}.
                  </h1>
                  <div className="grid w-full max-w-sm gap-3 text-left" aria-live="polite">
                    {PREFLIGHT_STEPS.map((label, index) => {
                      const complete = index < preflightIndex;
                      const active = index === preflightIndex;

                      return (
                        <div
                          key={label}
                          className={`grid grid-cols-[1.25rem_1fr_auto] items-center gap-3 transition-colors duration-200 ${
                            complete || active ? "text-foreground" : "text-muted-foreground"
                          }`}
                        >
                          <span
                            className={`grid size-5 place-items-center rounded-full border ${
                              complete
                                ? "border-primary bg-primary text-primary-foreground"
                                : active
                                  ? "border-primary"
                                  : "border-border"
                            }`}
                            aria-hidden
                          >
                            {complete ? (
                              <CheckIcon className="size-3" />
                            ) : active ? (
                              <span className="size-1.5 animate-pulse rounded-full bg-primary" />
                            ) : null}
                          </span>
                          <span className="type-body">{label}</span>
                          <span className="type-caption">
                            {complete ? "Done" : active ? "Working" : "Waiting"}
                          </span>
                        </div>
                      );
                    })}
                  </div>
                </section>
              ) : null}

              {stage === "payment" ? (
                <section
                  className="grid w-full max-w-xl justify-items-center gap-8 text-center"
                  aria-labelledby="onboarding-payment-title"
                >
                  <h1
                    id="onboarding-payment-title"
                    className={`max-w-xl ${HEADLINE_CLASS}`}
                  >
                    {agentName} is ready to wake up.
                  </h1>

                  <Card className="w-full max-w-md rounded-[var(--radius-dialog)] py-6 text-left ring-0 sm:py-8">
                    <CardContent className="grid gap-6 px-6 sm:px-8">
                      <div>
                        <p className="type-body-sm text-muted-foreground">Finite hosted agent</p>
                        <p className="mt-1 font-sans text-4xl leading-none font-medium tracking-normal">
                          $200{" "}
                          <span className="type-body text-muted-foreground">USD / month</span>
                        </p>
                      </div>

                      <Button
                        type="button"
                        size="xl"
                        className="w-full"
                        onClick={() => setStage("ready")}
                      >
                        <CreditCardIcon />
                        Continue with Stripe
                      </Button>

                      {showLaunchCode ? (
                        <div className="grid grid-cols-[1fr_auto] gap-2">
                          <Label htmlFor="preview-launch-code" className="sr-only">
                            Launch Code
                          </Label>
                          <Input
                            id="preview-launch-code"
                            placeholder="Launch Code"
                            autoFocus
                            className="h-12"
                          />
                          <Button type="button" size="xl" variant="outline">
                            Apply
                          </Button>
                        </div>
                      ) : (
                        <Button
                          type="button"
                          size="xl"
                          variant="ghost"
                          className="justify-self-center"
                          onClick={() => setShowLaunchCode(true)}
                        >
                          <KeyRoundIcon />
                          Use a Launch Code
                        </Button>
                      )}
                    </CardContent>
                  </Card>
                </section>
              ) : null}

              {stage === "ready" ? (
                <section
                  className="grid w-full max-w-xl justify-items-center gap-7 text-center"
                  aria-labelledby="onboarding-ready-title"
                >
                  <h1 id="onboarding-ready-title" className={HEADLINE_CLASS}>
                    {agentName} is online.
                  </h1>
                  <p className="type-body-lg text-muted-foreground">
                    Go introduce yourself to your new Finite Agent.
                  </p>
                  <Button type="button" size="xl">
                    Meet {agentName}
                  </Button>
                </section>
              ) : null}
            </div>
          </div>
        </main>
      </div>
    </div>
  );
}

function StageVisual({ stage, agentName }: { stage: Stage; agentName: string }) {
  return (
    <div className="grid h-32 place-items-center">
      {stage === "preflight" ? (
        <FiniteLoader
          label={`Preparing ${agentName}`}
          size={72}
          variant="center-out"
          className="text-primary"
        />
      ) : stage === "ready" ? (
        <StatusPrism state="happy" className="cursor-default" />
      ) : (
        <span
          className="size-[72px] bg-primary"
          aria-hidden
          style={{
            mask: 'url("/finite-icon.svg") center / contain no-repeat',
            WebkitMask: 'url("/finite-icon.svg") center / contain no-repeat',
          }}
        />
      )}
    </div>
  );
}
