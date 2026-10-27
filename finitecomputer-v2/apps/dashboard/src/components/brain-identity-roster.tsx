"use client";
import { useCallback, useMemo, useState } from "react";
import { KeyAliasDialog, type AliasTarget } from "@/components/brain-key-alias-dialog";
import { Button } from "@/components/ui/button";
import { useAgentInventory } from "@/hooks/use-agent-inventory";
import { useKeyAliases } from "@/hooks/use-key-aliases";
import { readBrainIdentities, type BrainIdentity } from "@/lib/brain-identity-roster";
import type { BrainRow } from "@/lib/agent-product-inventory";
import tableStyles from "@/styles/dashboard-table.module.css";

const roles: Record<string, string> = {
  owner: "Owner", personalAgent: "Personal agent", admin: "Admin", member: "Member", guest: "Guest",
  mountParticipant: "Shared folder", retainedMountAccess: "Removed", noCurrentRole: "Removed",
};
const folderStates: Record<string, string> = { grantMissing: "key pending", revocationIncomplete: "removal pending" };

/** What Finite knows about this key, without any alias. */
function actualLabel(row: BrainIdentity) {
  if (row.kind === "agent") return row.name ? `${row.name} · Agent` : null;
  return [row.name, row.email].filter(Boolean).join(" · ") || null;
}

function shortKey(npub: string) {
  return `${npub.slice(0, 12)}…${npub.slice(-6)}`;
}

function Identity({ row, alias }: { row: BrainIdentity; alias: string | null }) {
  if (alias) {
    return <>
      <span>{alias}</span>{" "}
      <span className="ml-1 inline-block rounded-full border px-1.5 text-[11px] font-normal leading-4 text-muted-foreground align-middle">Your alias</span>
      <span className="block text-xs font-normal text-muted-foreground">{actualLabel(row) ?? `Unknown · ${shortKey(row.npub)}`}</span>
    </>;
  }
  const detail = row.kind === "agent" ? "Agent" : row.name ? row.email : null;
  const primary = row.kind === "agent" ? row.name : row.name ?? row.email;
  if (!primary) return <span className="text-muted-foreground">Unknown</span>;
  return <>{primary}{detail && <span className="block text-xs text-muted-foreground">{detail}</span>}</>;
}

export function BrainIdentityRoster({ runtimeId, brain }: { runtimeId: string; brain: BrainRow }) {
  const read = useCallback((id: string, signal: AbortSignal) => readBrainIdentities(id, brain.id, signal), [brain.id]);
  const { data, busy } = useAgentInventory(runtimeId, read);
  const npubs = useMemo(() => data?.map(row => row.npub) ?? null, [data]);
  const aliases = useKeyAliases(npubs);
  const [editing, setEditing] = useState<AliasTarget | null>(null);
  const folderNames = new Map(brain.folders?.map(folder => [folder.id, folder.name]));
  return <section aria-busy={busy} className="mt-8 space-y-3">
    <div><h2 className="text-lg font-semibold">{brain.name}</h2>
      <p className="text-sm text-muted-foreground">{data ? `${data.length} identities` : busy ? "Loading identities…" : "Identities unavailable"}</p>
      {data && data.length > 0 && aliases.status === "unavailable" && <p className="text-xs text-muted-foreground">Your private aliases are unavailable right now.</p>}
    </div>
    {data && data.length > 0 && <div className={`brain-table-scroll ${tableStyles.surface}`} role="region" aria-label={`Identities in ${brain.name}`} tabIndex={0}>
      <table className="brain-table">
        <thead><tr><th scope="col">Identity</th><th scope="col" className="w-24"><span className="sr-only">Your alias</span></th><th scope="col">Owner</th><th scope="col">Role</th><th scope="col">Folders</th><th scope="col">Key</th></tr></thead>
        <tbody>{data.map(row => {
          const alias = aliases.aliasFor(row.npub);
          const name = alias ?? (row.kind === "agent" ? row.name : row.name ?? row.email) ?? `unknown key ${shortKey(row.npub)}`;
          return <tr key={row.npub}>
            <th scope="row"><Identity row={row} alias={alias} /></th>
            <td>{aliases.status === "ready" && <Button type="button" variant="ghost" size="xs"
              aria-label={`${alias ? "Edit" : "Add"} alias for ${name}`}
              onClick={() => setEditing({ npub: row.npub, alias, actual: actualLabel(row) })}>
              {alias ? "Edit alias" : "Add alias"}
            </Button>}</td>
            <td>{row.ownerEmail}</td>
            <td>{roles[row.role] ?? row.role}</td>
            <td>{row.folders.map(folder => `${folderNames.get(folder.id) ?? folder.id}${folderStates[folder.state] ? ` (${folderStates[folder.state]})` : ""}`).join(", ")}</td>
            <td><span className="font-mono text-xs" title={row.npub}>{shortKey(row.npub)}</span></td>
          </tr>;
        })}</tbody>
      </table>
    </div>}
    {editing && <KeyAliasDialog key={editing.npub} target={editing}
      otherAliases={(data ?? []).filter(row => row.npub !== editing.npub).flatMap(row => aliases.aliasFor(row.npub) ?? [])}
      onSave={aliases.save} onClear={aliases.clear} onClose={() => setEditing(null)} />}
  </section>;
}
