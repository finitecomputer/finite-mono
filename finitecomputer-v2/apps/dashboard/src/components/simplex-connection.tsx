"use client";

import { useState } from "react";
import { LockKeyholeIcon } from "lucide-react";
import { ConnectionCard } from "@/components/connection-card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { AgentConnectionAction, SimplexConnectionStatus } from "@/lib/hosted-agent-controls";

export function SimplexConnection({ status, loaded, busy, mutate, refresh }: {
  status?: SimplexConnectionStatus;
  loaded: boolean;
  busy: boolean;
  mutate: (label: string, action: AgentConnectionAction) => Promise<void>;
  refresh: () => Promise<void>;
}) {
  const [code, setCode] = useState("");
  const [copied, setCopied] = useState(false);
  const paired = Boolean(status?.approved.length);
  return <ConnectionCard
    name="SimpleX"
    description="A private conversation with your agent. Pair your phone to get started."
    icon={<LockKeyholeIcon className="size-5" />}
    state={!status ? "unavailable" : status.enabled && status.ready && paired ? "connected" : "disconnected"}
    account={status?.enabled ? paired ? status.approved.map(p => p.name || `Contact ${p.user_id}`).join(", ") : "Waiting for pairing" : null}
    error={loaded && !status ? "Update your agent runtime to set up SimpleX." : status?.enabled && !status.ready ? "SimpleX is starting or temporarily unavailable. Refresh to check again." : null}
    footer={status?.enabled ? <div className="space-y-4">
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
          <p className="text-muted-foreground">Your agent sends a pairing code. Enter it below to approve this contact.</p>
        </div>
      </div> : <Button disabled={busy} onClick={() => void mutate("simplex", { action: "simplex_connect" })}>Get pairing QR</Button>}
      <form className="flex flex-wrap items-center gap-2" onSubmit={event => {
        event.preventDefault();
        void mutate("simplex", { action: "simplex_approve", code });
      }}>
        <Input aria-label="SimpleX pairing code" placeholder="Pairing code" value={code}
          onChange={event => setCode(event.target.value.toUpperCase())} maxLength={8}
          autoComplete="off" spellCheck={false} className="max-w-48 font-mono" />
        <Button disabled={busy || !/^[ABCDEFGHJKLMNPQRSTUVWXYZ23456789]{8}$/u.test(code)} type="submit">Approve contact</Button>
        <Button type="button" variant="outline" disabled={busy} onClick={() => void refresh()}>Refresh</Button>
      </form>
      {paired ? <p className="text-sm text-muted-foreground">Contact approved. Send another message in SimpleX to test the conversation.</p> : null}
      <p className="text-xs text-muted-foreground">Disconnect pauses SimpleX. Your identity, contacts, and history are retained.</p>
    </div> : null}
  >
    <Button variant={status?.enabled ? "outline" : "default"} disabled={busy || !status}
      onClick={() => void mutate("simplex", { action: status?.enabled ? "simplex_disconnect" : "simplex_connect" })}>
      {status?.enabled ? "Disconnect" : "Connect"}
    </Button>
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
