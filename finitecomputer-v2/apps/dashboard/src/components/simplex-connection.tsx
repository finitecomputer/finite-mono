"use client";

import { useState } from "react";
import { LockKeyholeIcon } from "lucide-react";
import { ConnectionCard } from "@/components/connection-card";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import type { AgentConnectionAction, SimplexConnectionStatus } from "@/lib/hosted-agent-controls";

export function SimplexConnection({ status, loaded, busy, mutate, refresh }: {
  status?: SimplexConnectionStatus;
  loaded: boolean;
  busy: boolean;
  mutate: (label: string, action: AgentConnectionAction) => Promise<void>;
  refresh: () => Promise<void>;
}) {
  const [copied, setCopied] = useState(false);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);
  const paired = Boolean(status?.approved.length);
  const canDisconnect = status?.enabled || status?.reset_pending;
  return <ConnectionCard
    name="SimpleX"
    description={paired ? "A private conversation with your agent." : "A private conversation with your agent. Pair your phone to get started."}
    icon={<LockKeyholeIcon className="size-5" />}
    state={!status ? "unavailable" : status.enabled && status.ready && paired ? "connected" : "disconnected"}
    account={status?.enabled ? paired ? status.approved.map(p => p.name || `Contact ${p.user_id}`).join(", ") : "Waiting for approval" : null}
    error={status?.reset_pending ? "Disconnect is unfinished. Retry to finish clearing this connection." : loaded && !status ? "Update your agent runtime to set up SimpleX." : status?.enabled && !status.ready ? "SimpleX is starting or temporarily unavailable. Refresh to check again." : null}
    footer={status?.enabled && !paired ? <div className="space-y-4">
      {status.address ? <div className="flex flex-col gap-5 sm:flex-row sm:items-center">
        <SimplexQr rows={status.qr} />
        <div className="space-y-3 text-sm">
          <p>Scan with SimpleX, connect, then send your agent a message.</p>
          <div className="flex flex-wrap gap-2">
            <Button asChild variant="outline" size="sm"><a href={status.address} target="_blank" rel="noreferrer">Open in SimpleX</a></Button>
            <Button variant="outline" size="sm" onClick={async () => {
              try { await navigator.clipboard.writeText(status.address!); setCopied(true); }
              catch { setCopied(false); }
            }}>{copied ? "Copied" : "Copy link"}</Button>
          </div>
          <p className="text-muted-foreground">Return here to approve your connection request. No pairing code is needed.</p>
        </div>
      </div> : <Button disabled={busy} onClick={() => void mutate("simplex", { action: "simplex_connect" })}>Get pairing QR</Button>}
      <div className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <h3 className="text-sm font-medium">Connection requests</h3>
          <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => void refresh()}>Refresh</Button>
        </div>
        {status.pending === undefined ? <p className="text-sm text-muted-foreground">Update your agent runtime to approve requests here.</p> : status.pending.length ? <>
          <p className="text-sm text-muted-foreground">Approve only the request you just made. Display names can be copied.</p>
          {status.pending.map(request => <div key={request.request_id} className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border p-3">
            <div className="min-w-0 text-sm">
              <p className="font-medium">{request.name || "Unnamed contact"}</p>
              <p className="text-muted-foreground">Contact ID: <span className="font-mono">{request.user_id}</span> · {request.age_minutes === 0 ? "Requested just now" : `Requested ${request.age_minutes} min ago`}</p>
            </div>
            <Button disabled={busy || !status.ready} onClick={() => void mutate("simplex", { action: "simplex_approve_request", request_id: request.request_id })}>Approve contact {request.user_id}</Button>
          </div>)}
        </> : <p className="text-sm text-muted-foreground">No pending requests. Connect in SimpleX and send a message; your request will appear here.</p>}
      </div>
    </div> : null}
  >
    <Button variant={canDisconnect ? "outline" : "default"} disabled={busy || !status}
      onClick={() => canDisconnect ? setConfirmDisconnect(true) : void mutate("simplex", { action: "simplex_connect" })}>
      {canDisconnect ? "Disconnect" : "Connect"}
    </Button>
    <Dialog open={confirmDisconnect} onOpenChange={setConfirmDisconnect}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Disconnect SimpleX?</DialogTitle>
          <DialogDescription>This deletes your agent’s SimpleX identity, contacts, approvals, and SimpleX conversation history. Messages on your phone remain. You’ll need to pair again to reconnect.</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => setConfirmDisconnect(false)}>Cancel</Button>
          <Button variant="destructive" disabled={busy} onClick={() => {
            setConfirmDisconnect(false);
            void mutate("simplex", { action: "simplex_reset" });
          }}>Disconnect</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </ConnectionCard>;
}

export function SimplexQr({ rows }: { rows: string[] }) {
  if (!rows.length) return null;
  const size = rows.length + 8;
  const path = rows.flatMap((row, y) => [...row].flatMap((cell, x) => cell === "1" ? [`M${x + 4} ${y + 4}h1v1h-1z`] : [])).join("");
  return <svg role="img" aria-label="Scan to connect with your agent in SimpleX" viewBox={`0 0 ${size} ${size}`}
    className="size-56 shrink-0 rounded-lg bg-white" shapeRendering="crispEdges">
    <rect width={size} height={size} fill="white" />
    <path d={path} fill="black" />
  </svg>;
}
