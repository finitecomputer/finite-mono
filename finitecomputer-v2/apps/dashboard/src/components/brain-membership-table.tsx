"use client";
import { ChevronDownIcon } from "lucide-react";
import type { BrainRow } from "@/lib/agent-product-inventory";
import tableStyles from "@/styles/dashboard-table.module.css";

export function BrainTable({ brains }: { brains: BrainRow[] }) {
  return (
    <div className={`brain-table-scroll ${tableStyles.surface}`} role="region" aria-label="Agent brains and invitations" tabIndex={0}>
      <table className="brain-table">
        <caption className="sr-only">Brains accessible to the selected agent and pending invitations</caption>
        <thead><tr><th scope="col">Brain</th><th scope="col">Type</th><th scope="col">Access</th><th scope="col">Folders shown</th></tr></thead>
        <tbody>{brains.map((brain) => <tr key={brain.id}>
          <th scope="row">{brain.name}</th>
          <td>{brain.kind}</td>
          <td>{brain.role}</td>
          <td>{brain.pending ? null : brain.folders === null ? <span>Folder details unavailable</span> : brain.folders.length === 0 ? "0" :
            <details className="brain-table-folders"><summary aria-label={`${brain.folders.length} folders in ${brain.name}`}>{brain.folders.length}<ChevronDownIcon className="size-3.5" aria-hidden /></summary>
              <ul>{brain.folders.map(folder => <li key={folder.id}>{folder.name}</li>)}</ul>
            </details>}
          </td>
        </tr>)}</tbody>
      </table>
    </div>
  );
}
