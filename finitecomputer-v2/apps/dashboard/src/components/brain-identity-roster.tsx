"use client";
import { useCallback } from "react";
import { useAgentInventory } from "@/hooks/use-agent-inventory";
import { readBrainIdentities, type BrainIdentity } from "@/lib/brain-identity-roster";
import type { BrainRow } from "@/lib/agent-product-inventory";
import tableStyles from "@/styles/dashboard-table.module.css";

const roles: Record<string, string> = {
  owner: "Owner", personalAgent: "Personal agent", admin: "Admin", member: "Member", guest: "Guest",
  mountParticipant: "Shared folder", retainedMountAccess: "Removed", noCurrentRole: "Removed",
};
const folderStates: Record<string, string> = { grantMissing: "key pending", revocationIncomplete: "removal pending" };

function Identity({ row }: { row: BrainIdentity }) {
  const detail = row.kind === "agent" ? "Agent" : row.name ? row.email : null;
  const primary = row.kind === "agent" ? row.name : row.name ?? row.email;
  if (!primary) return <span className="text-muted-foreground">Unknown</span>;
  return <>{primary}{detail && <span className="block text-xs text-muted-foreground">{detail}</span>}</>;
}

export function BrainIdentityRoster({ runtimeId, brain }: { runtimeId: string; brain: BrainRow }) {
  const read = useCallback((id: string, signal: AbortSignal) => readBrainIdentities(id, brain.id, signal), [brain.id]);
  const { data, busy } = useAgentInventory(runtimeId, read);
  const folderNames = new Map(brain.folders?.map(folder => [folder.id, folder.name]));
  return <section aria-busy={busy} className="mt-8 space-y-3">
    <div><h2 className="text-lg font-semibold">{brain.name}</h2>
      <p className="text-sm text-muted-foreground">{data ? `${data.length} identities` : busy ? "Loading identities…" : "Identities unavailable"}</p></div>
    {data && data.length > 0 && <div className={`brain-table-scroll ${tableStyles.surface}`} role="region" aria-label={`Identities in ${brain.name}`} tabIndex={0}>
      <table className="brain-table">
        <thead><tr><th scope="col">Identity</th><th scope="col">Owner</th><th scope="col">Role</th><th scope="col">Folders</th><th scope="col">Key</th></tr></thead>
        <tbody>{data.map(row => <tr key={row.npub}>
          <th scope="row"><Identity row={row} /></th>
          <td>{row.ownerEmail}</td>
          <td>{roles[row.role] ?? row.role}</td>
          <td>{row.folders.map(folder => `${folderNames.get(folder.id) ?? folder.id}${folderStates[folder.state] ? ` (${folderStates[folder.state]})` : ""}`).join(", ")}</td>
          <td><span className="font-mono text-xs" title={row.npub}>{`${row.npub.slice(0, 12)}…${row.npub.slice(-6)}`}</span></td>
        </tr>)}</tbody>
      </table>
    </div>}
  </section>;
}
