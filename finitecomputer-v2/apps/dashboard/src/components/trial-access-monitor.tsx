"use client";

import { useEffect } from "react";
import { usePathname, useRouter } from "next/navigation";
import { startTrialAccessMonitor } from "@/lib/trial-access-monitor";

/** Recheck an already-open dashboard, including a native chat frame. Server
 * routes independently authorize every new request; this is the visible UX. */
export function TrialAccessMonitor({ blocked, excludeHome = false }: { blocked: boolean; excludeHome?: boolean }) {
  const pathname = usePathname();
  const router = useRouter();
  useEffect(() => {
    // Home owns its monitor so its baseline matches the rendered billing
    // snapshot, rather than the layout's independent billing read.
    if (excludeHome && pathname === "/dashboard") return;
    return startTrialAccessMonitor({
      blocked,
      pathname,
      refresh: () => router.refresh(),
      replace: (path) => window.location.replace(path),
    });
  }, [blocked, excludeHome, pathname, router]);
  return null;
}
