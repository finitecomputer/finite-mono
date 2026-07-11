"use client";

import { RotateCcwIcon } from "lucide-react";
import { useParams } from "next/navigation";

import { Button } from "@/components/ui/button";

export default function HostedWebChatError({ reset }: { reset: () => void }) {
  const { machineId } = useParams<{ machineId: string }>();

  return (
    <section className="mx-auto max-w-2xl rounded-xl border border-destructive/30 bg-destructive/5 p-6">
      <h1 className="text-xl font-semibold">Chat couldn&apos;t load</h1>
      <p className="mt-2 text-sm text-muted-foreground">
        Your agent overview and recovery controls are still available. Retry chat, or
        return to the overview to restart or recover its chat services.
      </p>
      <div className="mt-5 flex flex-wrap gap-2">
        <Button type="button" onClick={reset}>
          <RotateCcwIcon />
          Retry
        </Button>
        <Button asChild variant="outline">
          <a href={`/dashboard/machines/${encodeURIComponent(machineId)}`}>Agent overview</a>
        </Button>
      </div>
    </section>
  );
}
