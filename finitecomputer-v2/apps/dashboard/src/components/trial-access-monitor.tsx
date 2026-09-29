"use client";

import { useEffect } from "react";
import { usePathname, useRouter } from "next/navigation";

/** Recheck an already-open dashboard, including a native chat frame. Server
 * routes independently authorize every new request; this is the visible UX. */
export function TrialAccessMonitor() {
  const pathname = usePathname();
  const router = useRouter();
  useEffect(() => {
    const controller = new AbortController();
    let pending = false;
    async function check() {
      if (pending) return;
      pending = true;
      try {
        const response = await fetch("/api/billing/trial-access", { cache: "no-store", signal: controller.signal });
        if (!response.ok) return;
        const status = await response.json();
        if (status.blocked) {
          if (pathname !== "/dashboard") window.location.replace("/dashboard");
          else router.refresh();
        }
      } catch { /* New agent requests still fail closed if Core is unavailable. */ }
      finally { pending = false; }
    }
    void check();
    const timer = window.setInterval(() => void check(), 30_000);
    return () => { window.clearInterval(timer); controller.abort(); };
  }, [pathname, router]);
  return null;
}
