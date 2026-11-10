"use client";

import { useEffect } from "react";
import { usePathname } from "next/navigation";
import { startFiniteChatMoveMonitor } from "@/lib/finite-chat-move-monitor";

/** Sends an already-open dashboard on to finite.chat once its person has
 * moved, rechecking on each navigation (FIN-463). */
export function FiniteChatMoveMonitor() {
  const pathname = usePathname();
  useEffect(() => {
    void pathname;
    return startFiniteChatMoveMonitor({ replace: (url) => window.location.replace(url) });
  }, [pathname]);
  return null;
}
