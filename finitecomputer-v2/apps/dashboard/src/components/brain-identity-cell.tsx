import type { BrainIdentity } from "@/lib/brain-identity-roster";

function Name({ primary, detail }: { primary: string; detail: string | null }) {
  return <>{primary}{detail && <span className="block text-xs text-muted-foreground">{detail}</span>}</>;
}

/** Core's resolved name or email first; else the public name Brain stored for this exact key; else Unknown. */
export function IdentityCell({ row }: { row: BrainIdentity }) {
  const primary = row.kind === "agent" ? row.name : row.name ?? row.email;
  if (primary) return <th scope="row"><Name primary={primary} detail={row.kind === "agent" ? "Agent" : row.name ? row.email : null} /></th>;
  if (row.publicName) return <th scope="row" title={row.publicNameStoredAt ? `Stored ${row.publicNameStoredAt}` : undefined}>
    <Name primary={row.publicName} detail="Public name" /></th>;
  return <th scope="row"><span className="text-muted-foreground">Unknown</span></th>;
}
