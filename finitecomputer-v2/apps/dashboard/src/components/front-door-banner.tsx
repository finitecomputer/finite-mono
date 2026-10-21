"use client";

import { useSyncExternalStore } from "react";
import { ArrowUpRightIcon, SparklesIcon, XIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  browserSessionStorage,
  createFrontDoorBannerStore,
} from "@/lib/front-door-banner";

const bannerStore = createFrontDoorBannerStore(browserSessionStorage);

/** Small, per-session dismissible pointer to the new Finite for accounts that
 * keep the legacy dashboard. Hidden during server render so a dismissed
 * banner never flashes back on hydration. */
export function FrontDoorBanner({ href }: { href: string }) {
  const dismissed = useSyncExternalStore(
    bannerStore.subscribe,
    bannerStore.dismissed,
    () => true
  );
  if (dismissed) return null;

  return (
    <aside
      aria-label="The new Finite"
      className="mb-4 flex items-center gap-3 rounded-xl border bg-card px-4 py-3 text-sm shadow-xs"
    >
      <SparklesIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
      <p className="min-w-0 flex-1">
        <strong className="font-medium">Try the new Finite.</strong>{" "}
        <span className="text-muted-foreground">
          Your agents here keep working while you look around.
        </span>
      </p>
      <Button asChild variant="outline" size="sm">
        <a href={href}>
          Open
          <ArrowUpRightIcon data-icon="inline-end" />
        </a>
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label="Dismiss for this session"
        onClick={bannerStore.dismiss}
      >
        <XIcon className="size-3.5 text-muted-foreground" />
      </Button>
    </aside>
  );
}
