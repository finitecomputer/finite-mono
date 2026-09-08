import { cn } from "@/lib/utils";

export function FinitePrivateUsageProgress({
  usedUnits,
  limitUnits,
  className,
}: {
  usedUnits: number;
  limitUnits: number;
  className?: string;
}) {
  const usedPercent = finitePrivateUsedPercent(usedUnits, limitUnits);

  return (
    <div className={cn("grid gap-2", className)}>
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1 text-xs">
        <span className="font-semibold tabular-nums text-foreground">{usedPercent}% used</span>
        <span className="tabular-nums text-muted-foreground">
          {formatWeightedTokens(usedUnits)} of {formatWeightedTokens(limitUnits)} weighted tokens
        </span>
      </div>
      <div
        className="h-2.5 overflow-hidden rounded-full bg-white/10 ring-1 ring-inset ring-border"
        role="progressbar"
        aria-label="Finite Private weighted token usage"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={usedPercent}
        aria-valuetext={`${formatWeightedTokens(usedUnits)} of ${formatWeightedTokens(limitUnits)} weighted tokens used`}
      >
        <div
          className={cn(
            "h-full rounded-full transition-[width] duration-200 ease-out motion-reduce:transition-none",
            usedPercent >= 90
              ? "bg-rose-400"
              : usedPercent >= 75
                ? "bg-amber-400"
                : "bg-emerald-400"
          )}
          style={{ width: `${usedPercent}%` }}
        />
      </div>
    </div>
  );
}

export function finitePrivateUsedPercent(usedUnits: number, limitUnits: number) {
  if (!Number.isFinite(usedUnits) || !Number.isFinite(limitUnits) || limitUnits <= 0) {
    return 0;
  }
  return Math.max(0, Math.min(100, Math.round((usedUnits / limitUnits) * 100)));
}

export function formatWeightedTokens(value: number) {
  return new Intl.NumberFormat("en-US").format(Math.max(0, value));
}
