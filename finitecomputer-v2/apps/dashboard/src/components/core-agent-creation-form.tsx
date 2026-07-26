"use client";

import { useEffect, useRef, useState } from "react";
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  CheckIcon,
  CreditCardIcon,
  ImagePlusIcon,
  KeyRoundIcon,
  RocketIcon,
  ShieldCheckIcon,
} from "lucide-react";

import { useAgentOnboardingStage } from "@/components/agent-onboarding-progress";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { FiniteLoader } from "@/components/finite-loader";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

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
  immersive = false,
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
  immersive?: boolean;
}) {
  const [step, setStep] = useState<"profile" | "access">("profile");
  const [displayName, setDisplayName] = useState(initialName ?? "");
  const [picturePreview, setPicturePreview] = useState(initialPictureUrl ?? "");
  const [showLaunchCode, setShowLaunchCode] = useState(false);
  const [hostingTier, setHostingTier] = useState<"standard" | "confidential">(
    allowConfidentialHosting && initialHostingTier === "confidential"
      ? "confidential"
      : "standard"
  );
  const [submitting, setSubmitting] = useState<"launch" | "stripe" | "launch-code" | null>(null);
  const submittedRef = useRef(false);
  const onboardingStage = useAgentOnboardingStage();
  const selectionRequiresAccess = requiresAccess || hostingTier === "confidential";
  const showLaunchCodeForm =
    showLaunchCode || !stripeConfigured || hostingTier === "confidential";

  useEffect(() => {
    return () => {
      if (picturePreview.startsWith("blob:")) URL.revokeObjectURL(picturePreview);
    };
  }, [picturePreview]);

  useEffect(() => {
    if (!immersive || !onboardingStage) return;
    onboardingStage.setStage(
      submitting && submitting !== "stripe" ? "launch" : step
    );
  }, [immersive, onboardingStage, step, submitting]);

  if (submitting) {
    if (immersive) {
      return (
        <div
          className="grid min-h-[32rem] w-full max-w-xl content-center justify-items-center gap-7 text-center"
          role="status"
          aria-live="polite"
        >
          <FiniteLoader
            label={
              submitting === "stripe"
                ? "Opening secure checkout"
                : `Creating ${displayName.trim() || "your agent"}`
            }
            size={72}
            variant="center-out"
          />
          <div className="grid gap-2">
            <strong className="font-sans text-3xl leading-tight font-medium tracking-[-0.02em] sm:text-5xl">
              {submitting === "stripe"
                ? "Opening secure checkout."
                : `Creating ${displayName.trim() || "your agent"}.`}
            </strong>
            <span className="type-body-lg text-muted-foreground">
              {submitting === "stripe"
                ? "You’ll return here after payment."
                : "We’ll keep this page updated while your agent starts."}
            </span>
          </div>
        </div>
      );
    }

    return (
      <div className="ocean-agent-spinup" role="status" aria-live="polite">
        <FiniteLoader
          label={
            submitting === "stripe"
              ? "Opening secure checkout"
              : "Creating your agent"
          }
          size={38}
          variant="center-out"
        />
        <div>
          <strong>{submitting === "stripe" ? "Opening secure checkout" : "Creating your agent"}</strong>
          <span>{submitting === "stripe" ? "You’ll return here after payment." : "We’ll take you to chat when it’s ready."}</span>
        </div>
      </div>
    );
  }

  return (
    <form
      action="/dashboard/agent-creation-requests"
      method="post"
      encType="multipart/form-data"
      className={
        immersive
          ? "grid w-full max-w-3xl gap-8"
          : "grid gap-5"
      }
      onSubmit={(event) => {
        if (submittedRef.current) {
          event.preventDefault();
          return;
        }
        submittedRef.current = true;
        const submitter = (event.nativeEvent as SubmitEvent).submitter as HTMLButtonElement | null;
        const access = submitter?.value;
        window.setTimeout(
          () => setSubmitting(access === "stripe" ? "stripe" : access === "launch-code" ? "launch-code" : "launch"),
          0
        );
      }}
    >
      <input type="hidden" name="idempotencyKey" value={idempotencyKey} />
      {returnMachineId ? <input type="hidden" name="machine" value={returnMachineId} /> : null}

      {!immersive ? (
        <div className="flex items-center gap-2 text-xs text-muted-foreground" aria-label="Setup progress">
          <StepDot active={step === "profile"} complete={step === "access"}>1</StepDot>
          <span className={step === "profile" ? "text-foreground" : undefined}>Profile</span>
          <span aria-hidden>—</span>
          <StepDot active={step === "access"}>2</StepDot>
          <span className={step === "access" ? "text-foreground" : undefined}>Access</span>
        </div>
      ) : null}

      {error ? (
        <p className="rounded-[var(--radius-card-inner)] border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}

      <div
        className={
          step === "profile"
            ? immersive
              ? "grid justify-items-center gap-7 text-center"
              : "grid gap-5"
            : "hidden"
        }
        aria-hidden={step !== "profile"}
      >
        {immersive ? (
          <div className="grid gap-2">
            <h1 className="font-sans text-3xl leading-tight font-medium tracking-[-0.02em] sm:text-5xl">
              Give your agent a name.
            </h1>
            <p className="type-body-lg text-muted-foreground">
              This is how they’ll introduce themselves.
            </p>
          </div>
        ) : null}

        <div className="flex items-center gap-4">
          <Avatar className="size-16" size="lg">
            {picturePreview ? <AvatarImage src={picturePreview} alt="Agent profile preview" /> : null}
            <AvatarFallback className="text-lg">{agentInitials(displayName)}</AvatarFallback>
          </Avatar>
          <div className="grid gap-1.5">
            <Label
              htmlFor="coreAgentPicture"
              className="inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-full border border-border px-3 text-sm font-medium hover:bg-muted"
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
              tabIndex={step === "profile" ? 0 : -1}
              onChange={(event) => {
                const file = event.currentTarget.files?.[0];
                if (!file) return;
                setPicturePreview((current) => {
                  if (current.startsWith("blob:")) URL.revokeObjectURL(current);
                  return URL.createObjectURL(file);
                });
              }}
            />
            <span className="text-xs text-muted-foreground">Optional · PNG, JPEG, WebP, or GIF</span>
          </div>
        </div>

        <div className={immersive ? "grid w-full max-w-md gap-2 text-left" : "grid gap-2"}>
          <Label htmlFor="coreAgentDisplayName">Agent name</Label>
          <Input
            className={immersive ? "h-12 text-base" : undefined}
            id="coreAgentDisplayName"
            name="displayName"
            value={displayName}
            onChange={(event) => setDisplayName(event.target.value)}
            placeholder="Moss"
            maxLength={80}
            required
            autoFocus
            tabIndex={step === "profile" ? 0 : -1}
          />
        </div>

        <fieldset className={immersive ? "grid w-full max-w-md gap-2 text-left" : "grid gap-2"}>
          <legend className="mb-1 text-sm font-medium text-foreground">Hosting</legend>
          <label
            className={`flex cursor-pointer items-start gap-3 rounded-[var(--radius-card-inner)] border p-3 ${
              hostingTier === "standard"
                ? "border-primary bg-primary/5"
                : "border-border bg-white/[0.03]"
            }`}
          >
            <input
              type="radio"
              name="hostingTier"
              value="standard"
              checked={hostingTier === "standard"}
              onChange={() => setHostingTier("standard")}
              className="mt-1"
              tabIndex={step === "profile" ? 0 : -1}
            />
            <span className="grid gap-0.5">
              <span className="text-sm font-medium text-foreground">Standard</span>
              <span className="text-xs text-muted-foreground">
                Finite-hosted private state · $200 USD / month
              </span>
            </span>
          </label>
          {allowConfidentialHosting ? (
            <label
              className={`flex cursor-pointer items-start gap-3 rounded-[var(--radius-card-inner)] border p-3 ${
                hostingTier === "confidential"
                  ? "border-primary bg-primary/5"
                  : "border-border bg-white/[0.03]"
              }`}
            >
              <input
                type="radio"
                name="hostingTier"
                value="confidential"
                checked={hostingTier === "confidential"}
                onChange={() => setHostingTier("confidential")}
                className="mt-1"
                tabIndex={step === "profile" ? 0 : -1}
              />
              <span className="grid gap-0.5">
                <span className="inline-flex items-center gap-1.5 text-sm font-medium text-foreground">
                  <ShieldCheckIcon className="size-4" />
                  Confidential
                  <span className="rounded-full border border-border px-2 py-0.5 text-[10px] font-normal text-muted-foreground">
                    Early access
                  </span>
                </span>
                <span className="text-xs text-muted-foreground">
                  Runs in confidential compute. Pardon our dust while access is limited to Confidential Launch Codes.
                </span>
              </span>
            </label>
          ) : null}
        </fieldset>

        {selectionRequiresAccess ? (
          <Button
            type="button"
            className="w-fit"
            size={immersive ? "xl" : "default"}
            disabled={!displayName.trim()}
            onClick={() => setStep("access")}
          >
            Continue
            <ArrowRightIcon />
          </Button>
        ) : (
          <Button
            type="submit"
            name="access"
            value="entitled"
            className="w-fit"
            size={immersive ? "xl" : "default"}
            disabled={!displayName.trim()}
          >
            <RocketIcon />
            Create agent
          </Button>
        )}
      </div>

      <div
        className={
          step === "access"
            ? immersive
              ? "grid justify-items-center gap-6 text-center"
              : "grid gap-5"
            : "hidden"
        }
        aria-hidden={step !== "access"}
      >
        <div className={immersive ? "grid gap-2" : undefined}>
          {immersive ? (
            <h1 className="font-sans text-3xl leading-tight font-medium tracking-[-0.02em] sm:text-5xl">
              {displayName.trim() || "Your agent"} is ready to launch.
            </h1>
          ) : (
            <h2 className="font-semibold text-foreground">
              Start {displayName.trim() || "your agent"}
            </h2>
          )}
          <p className="mt-1 text-sm text-muted-foreground">
            {hostingTier === "confidential"
              ? "Enter a Confidential Launch Code. Standard subscriptions do not unlock this option yet."
              : stripeConfigured
                ? "Pay securely or use a Launch Code."
                : "Enter your Launch Code."}
          </p>
        </div>

        <Card
          className={
            immersive
              ? "w-full max-w-xl border-border/80 bg-card/75 shadow-xl shadow-black/5 backdrop-blur"
              : "w-full border-0 bg-transparent py-0 shadow-none"
          }
        >
          <CardContent className={immersive ? "grid gap-6 p-6 sm:p-8" : "grid gap-5 px-0"}>
            {stripeConfigured && hostingTier === "standard" ? (
              <div
                className={
                  immersive
                    ? "grid justify-items-center gap-4"
                    : "grid gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-4"
                }
              >
                <div>
                  <div className="font-medium text-foreground">
                    Finite Computer Hosted Agent
                  </div>
                  <div className="text-2xl font-semibold text-foreground">
                    $200 USD{" "}
                    <span className="text-sm font-normal text-muted-foreground">
                      / month
                    </span>
                  </div>
                </div>
                <Button
                  type="submit"
                  name="access"
                  value="stripe"
                  className="w-fit"
                  size={immersive ? "xl" : "default"}
                >
                  <CreditCardIcon />
                  Continue to secure payment
                </Button>
                <p className="max-w-md text-xs text-muted-foreground">
                  Renews monthly until you cancel in the billing portal. Tax is added at
                  checkout where applicable, setup begins after payment, and refunds are
                  handled per our{" "}
                  <a
                    href="/privacy.txt"
                    target="_blank"
                    rel="noreferrer"
                    className="underline underline-offset-2 hover:text-foreground"
                  >
                    terms
                  </a>
                  .{" "}
                  <a
                    href="https://docs.google.com/forms/d/e/1FAIpQLSePGnux9EVHRGZf30q7MPEMdMmTb7djJxAPCM0hCf-wRTGv3w/viewform"
                    target="_blank"
                    rel="noreferrer"
                    className="underline underline-offset-2 hover:text-foreground"
                  >
                    Contact Finite
                  </a>{" "}
                  with questions.
                </p>
              </div>
            ) : null}

            {!immersive || showLaunchCodeForm ? (
              <div
                className={
                  immersive
                    ? "grid w-full gap-2 border-t border-border pt-6 text-left"
                    : "grid max-w-sm gap-2"
                }
              >
                <Label htmlFor="coreAgentLaunchCode">
                  {hostingTier === "confidential"
                    ? "Confidential Launch Code"
                    : "Launch Code"}
                </Label>
                <div className="flex gap-2">
                  <Input
                    className={immersive ? "h-11" : undefined}
                    id="coreAgentLaunchCode"
                    name="launchCode"
                    autoComplete="off"
                    placeholder="Enter code"
                    tabIndex={step === "access" ? 0 : -1}
                  />
                  <Button
                    type="submit"
                    name="access"
                    value="launch-code"
                    variant="outline"
                    size={immersive ? "lg" : "default"}
                  >
                    Apply
                  </Button>
                </div>
              </div>
            ) : (
              <Button
                type="button"
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

        <Button type="button" variant="ghost" className="w-fit" onClick={() => setStep("profile")}>
          <ArrowLeftIcon />
          Back
        </Button>
      </div>
    </form>
  );
}

function StepDot({
  active,
  complete = false,
  children,
}: {
  active: boolean;
  complete?: boolean;
  children: React.ReactNode;
}) {
  return (
    <span
      className={`inline-flex size-5 items-center justify-center rounded-full border text-[11px] ${
        active || complete
          ? "border-primary bg-primary text-primary-foreground"
          : "border-border text-muted-foreground"
      }`}
    >
      {complete ? <CheckIcon className="size-3" /> : children}
    </span>
  );
}

function agentInitials(name: string) {
  const initials = name
    .trim()
    .split(/\s+/u)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase())
    .join("");
  return initials || "✦";
}
