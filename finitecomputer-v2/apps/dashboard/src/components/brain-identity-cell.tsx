import type { BrainIdentity } from "@/lib/brain-identity-roster";

function Name({ primary, detail }: { primary: string; detail: string | null }) {
  return <>{primary}{detail && <span className="block text-xs text-muted-foreground">{detail}</span>}</>;
}

/** Current Core identity, dated scoped annotation, stored public name, then Unknown. */
export function IdentityCell({ row }: { row: BrainIdentity }) {
  const primary = row.kind === "agent" ? row.name : row.name ?? row.email;
  const agentDetail = row.lifecycle === "retired" ? "Agent (retired)" : row.lifecycle === "offboarding" ? "Agent (offboarding)" : "Agent";
  if (primary) return <th scope="row"><Name primary={primary} detail={row.kind === "agent" ? agentDetail : row.name ? row.email : null} /></th>;
  if (row.operatorAnnotation) {
    const annotation = row.operatorAnnotation;
    const kind = annotation.kind === "historical_identity" ? "Historical identity" : "Invitation key";
    return <th scope="row" title={`Evidence ${annotation.observedAt}`}><Name primary={annotation.label}
      detail={`${kind} · Evidence ${annotation.observedAt.slice(0, 10)}`} /></th>;
  }
  if (row.publicName) return <th scope="row" title={row.publicNameStoredAt ? `Stored ${row.publicNameStoredAt}` : undefined}>
    <Name primary={row.publicName} detail="Public name" /></th>;
  return <th scope="row"><span className="text-muted-foreground">Unknown</span></th>;
}
