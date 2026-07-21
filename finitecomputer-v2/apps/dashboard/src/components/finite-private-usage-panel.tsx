"use client";

import { RotateCcwIcon, ShieldCheckIcon } from "lucide-react";
import { useActionState } from "react";

import {
  claimFinitePrivateDailyResetAction,
  type FinitePrivateDailyResetActionState,
} from "@/app/actions";
import { FormActionButton } from "@/components/form-action-button";
import type { CoreFinitePrivateUsageStatus } from "@/lib/core-client";

const INITIAL_STATE: FinitePrivateDailyResetActionState = {
  message: "",
  tone: null,
};

function utcLabel(value: string) {
  const parsed = new Date(value);
  return Number.isNaN(parsed.valueOf()) ? value : parsed.toISOString().replace(".000Z", "Z");
}

export function FinitePrivateUsagePanel({
  usage,
}: {
  usage: CoreFinitePrivateUsageStatus;
}) {
  const [state, formAction] = useActionState(
    claimFinitePrivateDailyResetAction,
    INITIAL_STATE
  );
  const remainingPercent = Math.max(
    0,
    Math.min(100, Math.floor((usage.burstRemainingUnits / usage.burstLimitUnits) * 100))
  );

  return (
    <section id="finite-private" className="ocean-utility-card scroll-mt-20">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          <ShieldCheckIcon className="size-5" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="ocean-utility-card__title">Finite Private usage</h2>
          <p className="text-sm text-muted-foreground">
            {remainingPercent}% remains in your account-wide burst window. Resets at{" "}
            <time dateTime={usage.burstResetAt}>{utcLabel(usage.burstResetAt)}</time>.
          </p>
        </div>
        <form action={formAction}>
          <FormActionButton
            type="submit"
            variant="outline"
            pendingLabel="Resetting…"
            disabled={!usage.freeDailyResetAvailable}
          >
            <RotateCcwIcon />
            {usage.freeDailyResetAvailable ? "Use free daily reset" : "Free reset used today"}
          </FormActionButton>
        </form>
      </div>
      {!usage.freeDailyResetAvailable ? (
        <p className="mt-3 text-sm text-muted-foreground">
          Your next free reset is available at{" "}
          <time dateTime={usage.freeDailyResetAvailableAgainAt}>
            {utcLabel(usage.freeDailyResetAvailableAgainAt)}
          </time>
          .
        </p>
      ) : null}
      {state.message ? (
        <p
          className={`mt-3 text-sm ${
            state.tone === "error" ? "text-destructive" : "text-muted-foreground"
          }`}
          aria-live="polite"
        >
          {state.message}
        </p>
      ) : null}
    </section>
  );
}

export function FinitePrivateUsageUnavailablePanel({ error }: { error: string }) {
  return (
    <section id="finite-private" className="ocean-utility-card scroll-mt-20">
      <div className="ocean-utility-card__header">
        <span className="ocean-utility-card__icon" aria-hidden>
          <ShieldCheckIcon className="size-5" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="ocean-utility-card__title">Finite Private usage</h2>
          <p className="text-sm text-destructive" role="status">
            Usage controls are temporarily unavailable. {error}
          </p>
        </div>
      </div>
    </section>
  );
}
