"use client";

import { useEffect, useRef } from "react";
import { PanelLeftIcon } from "lucide-react";

import {
  brainClientPath,
  BRAIN_SESSION_PROOF_RESPONSE,
  parseBrainSessionProofRequest,
} from "@/lib/brain-session-bridge";

export function BrainHeader() {
  return (
    <header className="finite-brain-page__header">
      <button
        type="button"
        className="ocean-icon-button finite-brain-page__sidebar-toggle"
        aria-label="Open agent navigation"
        onClick={() => window.dispatchEvent(new Event("finite:open-agent-sidebar"))}
      >
        <PanelLeftIcon className="size-4" />
      </button>
      <strong>Brain</strong>
    </header>
  );
}

export function BrainFrame({
  title,
  agentEmail,
  agentName,
  agentNpub,
}: {
  title: string;
  agentEmail?: string | null;
  agentName?: string | null;
  agentNpub?: string | null;
}) {
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
    <div className="finite-brain-page__frame">
      <iframe
        ref={frameRef}
        className="size-full border-0"
        src={brainClientPath({ email: agentEmail, name: agentName, npub: agentNpub })}
        title={title}
        allow="clipboard-read; clipboard-write"
        sandbox="allow-downloads allow-forms allow-scripts"
        data-finite-brain-frame
      />
    </div>
  );
}
