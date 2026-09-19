"use client";

import { useEffect, useRef, useState } from "react";
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  ImagePlusIcon,
  LockKeyholeIcon,
} from "lucide-react";

import { useAgentOnboardingStage } from "@/components/agent-onboarding-progress";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { FiniteLoader } from "@/components/finite-loader";
import { LogoIcon } from "@/components/logo";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const headingClass =
  "outline-none text-balance font-sans text-3xl leading-tight font-medium tracking-[-0.02em] sm:text-5xl";
type Step = "code" | "billing" | "profile";
type Access = "launch-code" | "stripe" | "entitled";

export function CoreAgentCreationForm({
  error,
  idempotencyKey,
  initialName,
  initialPictureUrl,
  initialHostingTier,
  allowConfidentialHosting,
  returnMachineId,
  requiresAccess,
  stripeConfigured,
}: {
  error: string | null;
  idempotencyKey: string;
  initialName?: string | null;
  initialPictureUrl?: string | null;
  initialHostingTier?: "standard" | "confidential" | null;
  allowConfidentialHosting: boolean;
  returnMachineId?: string | null;
  requiresAccess: boolean;
  stripeConfigured: boolean;
}) {
  const [step, setStep] = useState<Step>("code");
  const [access, setAccess] = useState<Access>("launch-code");
  const [code, setCode] = useState("");
  const [displayName, setDisplayName] = useState(initialName ?? "");
  const [picturePreview, setPicturePreview] = useState(initialPictureUrl ?? "");
  const [hostingTier, setHostingTier] = useState<"standard" | "confidential">(
    allowConfidentialHosting && initialHostingTier === "confidential"
      ? "confidential"
      : "standard",
  );
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
  const canUseEntitlement = !requiresAccess && hostingTier === "standard";
  const canPay = stripeConfigured && hostingTier === "standard";

  useEffect(() => {
    return () => {
      if (picturePreview.startsWith("blob:"))
        URL.revokeObjectURL(picturePreview);
    };
  }, [picturePreview]);

  useEffect(() => {
    heading.current?.focus();
  }, [step]);
  useEffect(() => {
    setStage?.(
      submitting ? (access === "stripe" ? "billing" : "launch") : step,
    );
  }, [setStage, step, submitting, access]);

  function chooseAccess(selected: Access) {
    setAccess(selected);
    setStep("billing");
  }

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
              ? "You’ll return here after payment."
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
        // Enter advances the visible step, never silently chooses payment or
        // redeems a code before the person has named and confirmed their agent.
        if (step !== "profile") {
          event.preventDefault();
          if (step === "code" && code.trim()) chooseAccess("launch-code");
          else if (step === "billing") setStep("profile");
          return;
        }
        if (submittedRef.current || !nameIsValid) {
          event.preventDefault();
          return;
        }
        submittedRef.current = true;
        // Keep all successful controls mounted until the browser captures the
        // native multipart submission, including an optional profile image.
        window.setTimeout(() => setSubmitting(true), 0);
      }}
    >
      <input type="hidden" name="idempotencyKey" value={idempotencyKey} />
      <input type="hidden" name="access" value={access} />
      <input
        type="hidden"
        name="launchCode"
        value={access === "launch-code" ? code.trim() : ""}
      />
      <input type="hidden" name="hostingTier" value={hostingTier} />
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

      {step === "code" ? (
        <>
          <div className="grid gap-3">
            <h1 ref={heading} tabIndex={-1} className={headingClass}>
              Do you have a launch code?
            </h1>
            <p className="type-body-lg text-pretty text-muted-foreground">
              {canUseEntitlement
                ? "Use a code, or continue with your account’s access."
                : canPay
                  ? "Use a code, or continue with a monthly plan."
                  : "Enter your code to set up your agent."}
            </p>
          </div>
          {allowConfidentialHosting ? (
            <fieldset className="flex flex-wrap justify-center gap-5 text-sm">
              <legend className="sr-only">Hosting</legend>
              <label className="flex min-h-10 items-center gap-2">
                <input
                  type="radio"
                  checked={hostingTier === "standard"}
                  onChange={() => setHostingTier("standard")}
                />
                Standard
              </label>
              <label className="flex min-h-10 items-center gap-2">
                <input
                  type="radio"
                  checked={hostingTier === "confidential"}
                  onChange={() => setHostingTier("confidential")}
                />
                Confidential · Early access
              </label>
            </fieldset>
          ) : null}
          {hostingTier === "confidential" ? (
            <p className="max-w-md text-sm text-muted-foreground">
              Enter a Confidential Launch Code. Standard subscriptions do not
              unlock this option yet.
            </p>
          ) : null}
          <div className="grid w-full max-w-md gap-2 text-left">
            <Label htmlFor="coreAgentLaunchCode">
              {hostingTier === "confidential"
                ? "Confidential Launch Code"
                : "Launch Code"}
            </Label>
            <Input
              id="coreAgentLaunchCode"
              className="h-12 text-base"
              value={code}
              autoComplete="off"
              spellCheck={false}
              placeholder="finite-xxxxxxx"
              onChange={(event) => setCode(event.target.value)}
              aria-describedby="code-help"
            />
            <p id="code-help" className="text-sm text-muted-foreground">
              We’ll validate your code when you launch. No payment is taken on
              this path.
            </p>
          </div>
          <Button
            type="button"
            size="xl"
            disabled={!code.trim()}
            onClick={() => chooseAccess("launch-code")}
          >
            Continue with code <ArrowRightIcon />
          </Button>
          {canUseEntitlement ? (
            <Button
              type="button"
              size="xl"
              variant="outline"
              onClick={() => chooseAccess("entitled")}
            >
              Use account access <ArrowRightIcon />
            </Button>
          ) : canPay ? (
            <Button
              type="button"
              size="xl"
              variant="outline"
              onClick={() => chooseAccess("stripe")}
            >
              Continue without a code <ArrowRightIcon />
            </Button>
          ) : null}
        </>
      ) : null}

      {step === "billing" ? (
        <>
          <h1 ref={heading} tabIndex={-1} className={headingClass}>
            {access === "launch-code"
              ? "Launch with your code."
              : access === "entitled"
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
                      {hostingTier === "confidential"
                        ? "Confidential Agent"
                        : "Hosted Agent"}
                    </h2>
                  </div>
                </div>
                <div className="grid gap-3 border-t border-border/70 pt-6">
                  <div className="flex flex-wrap items-baseline gap-2">
                    <span className="text-3xl font-semibold tracking-tight tabular-nums">
                      {access === "stripe"
                        ? "$200 USD"
                        : access === "launch-code"
                          ? "Launch code"
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
                      : access === "launch-code"
                        ? "Your code’s access is checked when you launch. If it cannot be used, you can try another code or choose a monthly plan."
                        : "Your available agent allowance will be checked when you launch."}
                  </p>
                </div>
                <Button
                  type="button"
                  size="xl"
                  className="w-full justify-between"
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
                added at checkout where applicable, setup begins after payment,
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
          <Button type="button" variant="ghost" onClick={() => setStep("code")}>
            <ArrowLeftIcon />
            Back
          </Button>
        </>
      ) : null}

      {/* Keep the file input mounted across steps so Back retains the upload. */}
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
        <div className="flex flex-wrap items-center justify-center gap-4">
          <Avatar className="size-12">
            {picturePreview ? (
              <AvatarImage src={picturePreview} alt="Agent profile preview" />
            ) : null}
            <AvatarFallback>
              {displayName.trim().slice(0, 2).toUpperCase() || "✦"}
            </AvatarFallback>
          </Avatar>
          <div className="grid gap-1.5 text-left">
            <Label
              htmlFor="coreAgentPicture"
              className="inline-flex min-h-10 cursor-pointer items-center gap-2 rounded-full border border-border px-4 text-sm hover:bg-muted"
            >
              <ImagePlusIcon className="size-4" />
              Choose picture
            </Label>
            <input
              id="coreAgentPicture"
              name="profilePicture"
              type="file"
              accept="image/png,image/jpeg,image/webp,image/gif"
              className="sr-only"
              onChange={(event) => {
                const file = event.currentTarget.files?.[0];
                if (file) setPicturePreview(URL.createObjectURL(file));
              }}
            />
            <span className="text-xs text-muted-foreground">
              Optional · Up to 5 MB
            </span>
          </div>
        </div>
        <Button type="submit" size="xl" disabled={!nameIsValid}>
          {access === "stripe" ? "Continue to secure payment" : "Launch agent"}
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
