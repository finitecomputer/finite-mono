import type { BrainIdentity } from "@/lib/brain-identity-roster";
import { unresolvedIdentityMessage } from "@/lib/brain-identity-resolution";

/** Current Core identity, dated scoped annotation, stored public name, then no label. */
export function identityLabel(row: BrainIdentity): { primary: string | null; detail: string | null; title?: string } {
  const primary = row.kind === "agent" ? row.name : row.name ?? row.email;
  const agentDetail = row.lifecycle === "retired" ? "Agent (retired)" : row.lifecycle === "offboarding" ? "Agent (offboarding)" : "Agent";
  if (primary) return { primary, detail: row.kind === "agent" ? agentDetail : row.name ? row.email : null };
  if (row.operatorAnnotation) {
    const annotation = row.operatorAnnotation;
    const kind = annotation.kind === "historical_identity" ? "Historical identity" : "Invitation key";
    return { primary: annotation.label, detail: `${kind} · Evidence ${annotation.observedAt.slice(0, 10)}`, title: `Evidence ${annotation.observedAt}` };
  }
  if (row.publicName) return { primary: row.publicName, detail: "Public name", title: row.publicNameStoredAt ? `Stored ${row.publicNameStoredAt}` : undefined };
  return { primary: null, detail: null };
}

export function IdentityCell({ row, alias }: { row: BrainIdentity; alias?: string | null }) {
  const { primary, detail, title } = identityLabel(row);
  const identity = primary
    ? <>{primary}{detail && <span className="block text-xs text-muted-foreground">{detail}</span>}</>
    : <span className="text-muted-foreground">{unresolvedIdentityMessage(row.resolution)}</span>;
  return <th scope="row" title={title}>{alias ? <>
    <span>{alias}</span>{" "}
    <span className="ml-1 inline-block rounded-full border px-1.5 text-[11px] font-normal leading-4 text-muted-foreground align-middle">Your alias</span>
    <span className="block text-xs font-normal text-muted-foreground">{identity}</span>
  </> : identity}</th>;
}
