"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";

type Props = {
  enabled: boolean;
  intervalMs?: number;
  maxIntervalMs?: number;
  // Optional hard stop (epoch ms). One final refresh still fires after the
  // deadline so the server can render its post-deadline state, then the
  // polling loop ends.
  deadlineAtMs?: number;
};

export function PendingRefresh({
  enabled,
  intervalMs = 3000,
  maxIntervalMs = 30_000,
  deadlineAtMs,
}: Props) {
  const router = useRouter();

  useEffect(() => {
    if (!enabled) {
      return;
    }

    let cancelled = false;
    let delayMs = intervalMs;
    let timer: number | null = null;

    const schedule = () => {
      if (deadlineAtMs !== undefined && Date.now() >= deadlineAtMs) {
        return;
      }
      timer = window.setTimeout(() => {
        if (cancelled) {
          return;
        }
        router.refresh();
        delayMs = Math.min(Math.round(delayMs * 1.5), maxIntervalMs);
        schedule();
      }, delayMs);
    };

    timer = window.setTimeout(() => {
      if (cancelled) {
        return;
      }
      router.refresh();
      schedule();
    }, intervalMs);

    return () => {
      cancelled = true;
      if (timer !== null) {
        window.clearTimeout(timer);
      }
    };
  }, [enabled, intervalMs, maxIntervalMs, deadlineAtMs, router]);

  return null;
}
