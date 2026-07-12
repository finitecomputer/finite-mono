"use client";

import { useEffect, useRef, useState } from "react";
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  CheckIcon,
  CreditCardIcon,
  ImagePlusIcon,
  Loader2Icon,
  RocketIcon,
} from "lucide-react";

import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function CoreAgentCreationForm({
  error,
  idempotencyKey,
  initialName,
  initialPictureUrl,
  returnMachineId,
  requiresAccess,
  stripeConfigured,
}: {
  error: string | null;
  idempotencyKey: string;
  initialName?: string | null;
  initialPictureUrl?: string | null;
  returnMachineId?: string | null;
  requiresAccess: boolean;
  stripeConfigured: boolean;
}) {
  const [step, setStep] = useState<"profile" | "access">("profile");
  const [displayName, setDisplayName] = useState(initialName ?? "");
  const [picturePreview, setPicturePreview] = useState(initialPictureUrl ?? "");
  const [submitting, setSubmitting] = useState<"launch" | "stripe" | "launch-code" | null>(null);
  const submittedRef = useRef(false);

  useEffect(() => {
    return () => {
      if (picturePreview.startsWith("blob:")) URL.revokeObjectURL(picturePreview);
    };
  }, [picturePreview]);

  if (submitting) {
    return (
      <div className="ocean-agent-spinup" role="status" aria-live="polite">
        <Loader2Icon className="size-5 animate-spin" aria-hidden />
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
      className="grid gap-5"
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

      <div className="flex items-center gap-2 text-xs text-muted-foreground" aria-label="Setup progress">
        <StepDot active={step === "profile"} complete={step === "access"}>1</StepDot>
        <span className={step === "profile" ? "text-foreground" : undefined}>Profile</span>
        <span aria-hidden>—</span>
        <StepDot active={step === "access"}>2</StepDot>
        <span className={step === "access" ? "text-foreground" : undefined}>Access</span>
      </div>

      {error ? (
        <p className="rounded-[var(--radius-card-inner)] border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}

      <div className={step === "profile" ? "grid gap-5" : "hidden"} aria-hidden={step !== "profile"}>
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

        <div className="grid gap-2">
          <Label htmlFor="coreAgentDisplayName">Agent name</Label>
          <Input
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

        <div className="flex items-center justify-between gap-3 rounded-[var(--radius-card-inner)] border border-border bg-white/[0.03] p-3">
          <div>
            <div className="text-sm font-medium text-foreground">Hosted by Finite</div>
            <div className="text-xs text-muted-foreground">Always available, with its own private state.</div>
          </div>
          <span className="inline-flex size-6 items-center justify-center rounded-full bg-primary text-primary-foreground">
            <CheckIcon className="size-3.5" />
          </span>
        </div>

        {requiresAccess ? (
          <Button
            type="button"
            className="w-fit"
            disabled={!displayName.trim()}
            onClick={() => setStep("access")}
          >
            Continue
            <ArrowRightIcon />
          </Button>
        ) : (
          <Button type="submit" name="access" value="entitled" className="w-fit" disabled={!displayName.trim()}>
            <RocketIcon />
            Create agent
          </Button>
        )}
      </div>

      <div className={step === "access" ? "grid gap-5" : "hidden"} aria-hidden={step !== "access"}>
        <div>
          <h2 className="font-semibold text-foreground">Start {displayName.trim() || "your agent"}</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {stripeConfigured ? "Pay securely or use a Launch Code." : "Enter your Launch Code."}
          </p>
        </div>

        {stripeConfigured ? (
          <Button type="submit" name="access" value="stripe" className="w-fit">
            <CreditCardIcon />
            Continue to payment
          </Button>
        ) : null}

        <div className="grid max-w-sm gap-2">
          <Label htmlFor="coreAgentLaunchCode">Launch Code</Label>
          <div className="flex gap-2">
            <Input
              id="coreAgentLaunchCode"
              name="launchCode"
              autoComplete="off"
              placeholder="Enter code"
              tabIndex={step === "access" ? 0 : -1}
            />
            <Button type="submit" name="access" value="launch-code" variant="outline">
              Apply
            </Button>
          </div>
        </div>

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
