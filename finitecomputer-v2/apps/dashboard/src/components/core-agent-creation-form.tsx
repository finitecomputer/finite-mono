"use client";

import { useEffect, useRef, useState } from "react";
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  LockKeyholeIcon,
} from "lucide-react";

import { useAgentOnboardingStage } from "@/components/agent-onboarding-progress";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { FiniteLoader } from "@/components/finite-loader";
import { LogoIcon } from "@/components/logo";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const headingClass =
  "outline-none text-balance font-sans text-3xl leading-tight font-medium tracking-[-0.02em] sm:text-5xl";
type Step = "billing" | "profile";
type Access = "stripe" | "entitled";

export function CoreAgentCreationForm({
  error,
  idempotencyKey,
  initialName,
  returnMachineId,
  requiresAccess,
  stripeConfigured,
  trialsEnabled = false,
}: {
  error: string | null;
  idempotencyKey: string;
  initialName?: string | null;
  returnMachineId?: string | null;
  requiresAccess: boolean;
  stripeConfigured: boolean;
  trialsEnabled?: boolean;
}) {
  const [step, setStep] = useState<Step>("billing");
  const access: Access = requiresAccess ? "stripe" : "entitled";
  const [trialCode, setTrialCode] = useState("");
  const [displayName, setDisplayName] = useState(initialName ?? "");
  const [submitting, setSubmitting] = useState(false);
  const submittedRef = useRef(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const setStage = useAgentOnboardingStage()?.setStage;
  const nameError = /[\u0000-\u001f\u007f]/u.test(displayName)
    ? "Choose a name without control characters."
    : null;
  const nameIsValid =
    Boolean(displayName.trim()) &&
    displayName.trim().length <= 80 &&
    !nameError;
  const canContinue = access === "entitled" || stripeConfigured;

  useEffect(() => {
    heading.current?.focus();
  }, [step]);
  useEffect(() => {
    setStage?.(
      submitting ? (access === "stripe" ? "billing" : "launch") : step,
    );
  }, [setStage, step, submitting, access]);

  if (submitting) {
    return (
      <section
        className="grid min-h-[28rem] w-full content-center justify-items-center gap-7 text-center"
        role="status"
        aria-live="polite"
      >
        <FiniteLoader
          label={
            access === "stripe"
              ? "Opening secure checkout"
              : "Submitting your launch"
          }
          size={72}
          variant="center-out"
        />
        <div className="grid gap-3">
          <h1 className={headingClass}>
            {access === "stripe"
              ? "Opening secure checkout."
              : `Launching ${displayName.trim()}.`}
          </h1>
          <p className="type-body-lg text-pretty text-muted-foreground">
            {access === "stripe"
              ? "You’ll return here after checkout."
              : "We’re checking your access and requesting your agent."}
          </p>
        </div>
      </section>
    );
  }

  return (
    <form
      action="/dashboard/agent-creation-requests"
      method="post"
      encType="multipart/form-data"
      className="grid w-full justify-items-center gap-7 text-center"
      onSubmit={(event) => {
        // Enter advances the plan only when checkout or account access is available.
        if (step !== "profile") {
          event.preventDefault();
          if (canContinue) setStep("profile");
          return;
        }
        if (submittedRef.current || !nameIsValid || !canContinue) {
          event.preventDefault();
          return;
        }
        submittedRef.current = true;
        // Keep all successful controls mounted until the browser captures the
        // native form submission.
        window.setTimeout(() => setSubmitting(true), 0);
      }}
    >
      <input type="hidden" name="idempotencyKey" value={idempotencyKey} />
      <input type="hidden" name="access" value={access} />
      <input type="hidden" name="trialCode" value={access === "stripe" ? trialCode.trim() : ""} />
      <input type="hidden" name="hostingTier" value="standard" />
      {returnMachineId ? (
        <input type="hidden" name="machine" value={returnMachineId} />
      ) : null}
      {error ? (
        <p
          role="alert"
          className="w-full max-w-md rounded-xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
        >
          {error}
        </p>
      ) : null}

      {step === "billing" ? (
        <>
          <h1 ref={heading} tabIndex={-1} className={headingClass}>
            {access === "entitled"
              ? "Use your account’s access."
              : "Your own Finite Agent."}
          </h1>
          <div className="grid w-full max-w-md gap-4">
            <Card className="gap-0 overflow-hidden rounded-2xl border-border/70 bg-card py-0 text-left shadow-sm">
              <CardContent className="grid gap-6 p-6 sm:p-8">
                <div className="flex items-center gap-3">
                  <div
                    className="flex size-11 shrink-0 items-center justify-center rounded-xl border border-border/60 bg-muted/50"
                    aria-hidden
                  >
                    <LogoIcon
                      className="text-2xl"
                      style={{
                        maskImage: 'url("/finite-onboarding-logo.svg")',
                        WebkitMaskImage: 'url("/finite-onboarding-logo.svg")',
                      }}
                    />
                  </div>
                  <div className="grid gap-0.5">
                    <span className="text-xs text-muted-foreground">
                      Finite Computer
                    </span>
                    <h2 className="text-base font-medium">
                      Hosted Agent
                    </h2>
                  </div>
                </div>
                <div className="grid gap-3 border-t border-border/70 pt-6">
                  <div className="flex flex-wrap items-baseline gap-2">
                    <span className="text-3xl font-semibold tracking-tight tabular-nums">
                      {access === "stripe"
                        ? "$200 USD"
                        : "Account access"}
                    </span>
                    {access === "stripe" ? (
                      <span className="text-sm text-muted-foreground">
                        / month
                      </span>
                    ) : null}
                  </div>
                  <p className="text-sm text-muted-foreground">
                    {access === "stripe"
                      ? "Billed monthly. Cancel anytime."
                      : "Your available agent allowance will be checked when you launch."}
                  </p>
                </div>
                {access === "stripe" && trialsEnabled ? <div className="grid gap-2">
                  <Label htmlFor="event-trial-code">Event trial code (optional)</Label>
                  <Input id="event-trial-code" value={trialCode} onChange={event => setTrialCode(event.target.value)} autoComplete="off" spellCheck={false} maxLength={128} placeholder="ABCD-EFGH-JKLM-NPQR" />
                  <p className="text-sm text-muted-foreground">A valid event code starts a free trial, normally seven days. Stripe will show your exact first billing date. A card is required; then $200/month plus applicable tax.</p>
                </div> : null}
                <Button
                  type="button"
                  size="xl"
                  className="w-full justify-between"
                  disabled={!canContinue}
                  onClick={() => setStep("profile")}
                >
                  Continue <ArrowRightIcon />
                </Button>
                {access === "stripe" ? (
                  <p className="flex items-center justify-center gap-1.5 text-xs text-muted-foreground">
                    <LockKeyholeIcon className="size-3.5" aria-hidden />
                    Secure checkout with Stripe after naming your agent
                  </p>
                ) : null}
              </CardContent>
            </Card>
            {access === "stripe" ? (
              <p className="px-3 text-xs leading-relaxed text-muted-foreground">
                Renews monthly until you cancel in the billing portal. Tax is
                added at checkout where applicable, setup begins after checkout,
                and refunds are handled per our{" "}
                <a
                  href="/privacy.txt"
                  target="_blank"
                  rel="noreferrer"
                  className="underline underline-offset-2"
                >
                  terms
                </a>
                .
              </p>
            ) : null}
          </div>
          {!canContinue ? (
            <p role="status" className="max-w-md text-sm text-muted-foreground">
              Signup is temporarily unavailable. Please try again later.
            </p>
          ) : null}
        </>
      ) : null}

      <div
        hidden={step !== "profile"}
        className={
          step === "profile"
            ? "grid w-full justify-items-center gap-7"
            : "hidden"
        }
      >
        <h1
          ref={step === "profile" ? heading : undefined}
          tabIndex={-1}
          className={headingClass}
        >
          Name your agent
        </h1>
        <div className="grid w-full max-w-md gap-2 text-left">
          <Label htmlFor="coreAgentDisplayName">Agent name</Label>
          <Input
            id="coreAgentDisplayName"
            name="displayName"
            className="h-12 text-base"
            value={displayName}
            onChange={(event) => setDisplayName(event.target.value)}
            placeholder="Moss"
            maxLength={80}
            required={step === "profile"}
            autoComplete="off"
            aria-invalid={Boolean(nameError)}
            aria-describedby={nameError ? "agent-name-error" : undefined}
          />
          {nameError ? (
            <p
              id="agent-name-error"
              role="status"
              className="text-sm text-destructive"
            >
              {nameError}
            </p>
          ) : null}
        </div>
        <Button type="submit" size="xl" disabled={!nameIsValid}>
          Continue
          <ArrowRightIcon />
        </Button>
        <Button
          type="button"
          variant="ghost"
          onClick={() => setStep("billing")}
        >
          <ArrowLeftIcon />
          Back
        </Button>
      </div>
    </form>
  );
}
