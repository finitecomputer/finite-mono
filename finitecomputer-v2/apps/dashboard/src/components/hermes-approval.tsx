"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";

export type HermesApproval = { request_id: string; command: string; choices: string[] };

export function HermesApprovalCard({ request, connected, respond }: {
  request: HermesApproval;
  connected: boolean;
  respond: (choice: "once" | "deny") => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function answer(choice: "once" | "deny") {
    setBusy(true);
    setError(null);
    try { await respond(choice); }
    catch (error) { setError(error instanceof Error ? error.message : "Could not send approval response."); }
    finally { setBusy(false); }
  }
  return <section aria-label="Command approval" className="mx-4 mb-3 rounded-lg border p-3 text-sm">
    <p className="font-medium">Hermes needs permission to run this command</p>
    <pre className="my-2 max-h-40 overflow-auto whitespace-pre-wrap break-all">{request.command}</pre>
    {error && <p role="alert" className="mb-2 text-destructive">{error}</p>}
    <div className="flex gap-2">
      {request.choices.includes("once") && <Button disabled={!connected || busy} onClick={() => void answer("once")}>Allow once</Button>}
      {request.choices.includes("deny") && <Button variant="outline" disabled={!connected || busy} onClick={() => void answer("deny")}>Deny</Button>}
    </div>
  </section>;
}
