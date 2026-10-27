"use client";
import { useCallback, useMemo, useRef, useState } from "react";
import { KeyAliasDialog, type AliasTarget } from "@/components/brain-key-alias-dialog";
import { IdentityCell, identityLabel } from "@/components/brain-identity-cell";
import {
  BrainInvitePanel, ChangeNotices, RemoveIdentityDialog, personLabel, runRemoval, useRosterChangeRunner,
} from "@/components/brain-roster-changes";
import { Button } from "@/components/ui/button";
import { useAgentInventory } from "@/hooks/use-agent-inventory";
import { useKeyAliases } from "@/hooks/use-key-aliases";
import { readBrainIdentities, type BrainIdentity } from "@/lib/brain-identity-roster";
import { shortNpub } from "@/lib/brain-roster-changes";
import type { BrainRow } from "@/lib/agent-product-inventory";
import tableStyles from "@/styles/dashboard-table.module.css";

const roles: Record<string, string> = {
  owner: "Owner", personalAgent: "Personal agent", admin: "Admin", member: "Member", guest: "Guest",
  mountParticipant: "Shared folder", retainedMountAccess: "Removed", noCurrentRole: "Removed",
};
const folderStates: Record<string, string> = { grantMissing: "key pending", revocationIncomplete: "removal pending" };

/** What Finite knows about this key, without any alias. */
function actualLabel(row: BrainIdentity) {
  const { primary, detail } = identityLabel(row);
  return [primary, detail].filter(Boolean).join(" · ") || null;
}

export function BrainIdentityRoster({ runtimeId, brain, agentName }: { runtimeId: string; brain: BrainRow; agentName: string }) {
  const read = useCallback((id: string, signal: AbortSignal) => readBrainIdentities(id, brain.id, signal), [brain.id]);
  const { data, busy, refresh } = useAgentInventory(runtimeId, read);
  const [revision, setRevision] = useState(0);
  const settled = useCallback(() => { setRevision(value => value + 1); void refresh(); }, [refresh]);
  const runner = useRosterChangeRunner(agentName, settled);
  const [removing, setRemoving] = useState<BrainIdentity | null>(null);
  const identities = data?.identities;
  const npubs = useMemo(() => identities?.map(row => row.npub) ?? null, [identities]);
  const aliases = useKeyAliases(npubs);
  const [editing, setEditing] = useState<AliasTarget | null>(null);
  const aliasTrigger = useRef<HTMLButtonElement | null>(null);
  const folderNames = new Map(brain.folders?.map(folder => [folder.id, folder.name]));
  // Only an Admin of an Organization Brain can change its roster here.
  const admin = brain.kind === "Organization" && brain.role === "Admin";
  const changes = admin ? data?.changes : undefined;
  const canRemove = !!changes?.actions.includes("remove");
  const removable = (row: BrainIdentity) => canRemove && (row.role === "member" || row.role === "admin")
    && row.npub !== changes?.actingNpub;
  return <section aria-busy={busy} className="mt-8 space-y-3">
    <div><h2 className="text-lg font-semibold">{brain.name}</h2>
      <p className="text-sm text-muted-foreground">{identities ? `${identities.length} identities` : busy ? "Loading identities…" : "Identities unavailable"}</p>
      {identities && identities.length > 0 && aliases.status === "unavailable" && <p className="text-xs text-muted-foreground">Your private aliases are unavailable right now.</p>}
    </div>
    {admin && data && changes === null && <p className="text-sm text-muted-foreground">Update {agentName} to invite or remove people here.</p>}
    {changes && changes.actions.length > 0 && <ChangeNotices progress={runner.progress} notice={runner.notice} />}
    {identities && identities.length > 0 && <div className={`brain-table-scroll ${tableStyles.surface}`} role="region" aria-label={`Identities in ${brain.name}`} tabIndex={0}>
      <table className="brain-table brain-identity-table">
        <thead><tr><th scope="col">Identity</th><th scope="col" className="w-24"><span className="sr-only">Your alias</span></th><th scope="col" className="w-48">Owner</th><th scope="col">Role</th><th scope="col">Folders</th><th scope="col">Key</th>
          {canRemove && <th scope="col" className="relative"><span className="sr-only">Actions</span></th>}</tr></thead>
        <tbody>{identities.map(row => {
          const alias = aliases.aliasFor(row.npub);
          const name = alias ?? identityLabel(row).primary ?? `unknown key ${shortNpub(row.npub)}`;
          return <tr key={row.npub}>
            <IdentityCell row={row} alias={alias} />
            <td>{aliases.status === "ready" && <Button type="button" variant="ghost" size="xs"
              aria-label={`${alias ? "Edit" : "Add"} alias for ${name}`}
              onClick={event => {
                aliasTrigger.current = event.currentTarget;
                setEditing({ npub: row.npub, alias, actual: actualLabel(row) });
              }}>
              {alias ? "Edit alias" : "Add alias"}
            </Button>}</td>
            <td>{row.kind === "agent" ? row.ownerEmail : null}</td>
            <td>{roles[row.role] ?? row.role}</td>
            <td>{row.folders.map(folder => `${folderNames.get(folder.id) ?? folder.id}${folderStates[folder.state] ? ` (${folderStates[folder.state]})` : ""}`).join(", ")}</td>
            <td><span className="font-mono text-xs" title={row.npub}>{shortNpub(row.npub)}</span></td>
            {canRemove && <td>{removable(row) && <Button type="button" variant="outline" size="sm" disabled={runner.busy}
              aria-label={`Remove ${personLabel(row)}`} onClick={() => setRemoving(row)}>Remove</Button>}</td>}
          </tr>;
        })}</tbody>
      </table>
    </div>}
    {changes?.actions.includes("invite") && <BrainInvitePanel runtimeId={runtimeId} brainId={brain.id} brainName={brain.name}
      agentName={agentName} runner={runner} revision={revision} />}
    <RemoveIdentityDialog row={removing} brainName={brain.name} agentName={agentName} onCancel={() => setRemoving(null)}
      onConfirm={row => { setRemoving(null); void runRemoval(runner, runtimeId, brain.id, agentName, row); }} />
    {editing && <KeyAliasDialog key={editing.npub} target={editing}
      otherAliases={(identities ?? []).filter(row => row.npub !== editing.npub).flatMap(row => aliases.aliasFor(row.npub) ?? [])}
      onSave={aliases.save} onClear={aliases.clear} onClose={() => setEditing(null)}
      onRestoreFocus={() => aliasTrigger.current?.focus()} />}
  </section>;
}
