"use client";

import { type ReactNode } from "react";
import { useRouter } from "next/navigation";

import { Button } from "@/components/ui/button";

// One-shot server re-render on demand: the manual counterpart to
// PendingRefresh for retry affordances.
export function RefreshButton({ children }: { children: ReactNode }) {
  const router = useRouter();

  return (
    <Button
      type="button"
      variant="outline"
      className="w-fit"
      onClick={() => router.refresh()}
    >
      {children}
    </Button>
  );
}
