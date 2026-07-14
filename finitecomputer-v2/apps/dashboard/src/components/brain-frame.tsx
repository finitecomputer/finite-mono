"use client";

import { useEffect, useRef } from "react";

import {
  BRAIN_SESSION_PROOF_RESPONSE,
  parseBrainSessionProofRequest,
} from "@/lib/brain-session-bridge";

export function BrainFrame({ title }: { title: string }) {
  const frameRef = useRef<HTMLIFrameElement>(null);

  useEffect(() => {
    let active = true;
    async function handleMessage(event: MessageEvent) {
      const frameWindow = frameRef.current?.contentWindow;
      if (!frameWindow || event.source !== frameWindow || event.origin !== "null") return;
      const proofRequest = parseBrainSessionProofRequest(event.data);
      if (!proofRequest) return;
      let proof: string | null = null;
      try {
        const response = await fetch("/api/brain/session-proof", {
          method: "POST",
          credentials: "same-origin",
          cache: "no-store",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ requestHash: proofRequest.requestHash }),
        });
        const body = (await response.json().catch(() => null)) as { proof?: unknown } | null;
        if (response.ok && typeof body?.proof === "string") proof = body.proof;
      } catch {
        proof = null;
      }
      if (!active || frameRef.current?.contentWindow !== frameWindow) return;
      frameWindow.postMessage(
        {
          type: BRAIN_SESSION_PROOF_RESPONSE,
          requestId: proofRequest.requestId,
          proof,
        },
        "*",
      );
    }
    window.addEventListener("message", handleMessage);
    return () => {
      active = false;
      window.removeEventListener("message", handleMessage);
    };
  }, []);

  return (
    <div className="h-[calc(100vh-12rem)] min-h-[36rem] overflow-hidden rounded-[var(--radius-card)] border border-border bg-card">
      <iframe
        ref={frameRef}
        className="size-full border-0"
        src="/client"
        title={title}
        allow="clipboard-read; clipboard-write"
        sandbox="allow-downloads allow-forms allow-scripts"
        data-finite-brain-frame
      />
    </div>
  );
}
