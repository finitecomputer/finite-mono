"use client";
import { useState } from "react";
import { Loader2Icon, TriangleAlertIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { BrainRow } from "@/lib/agent-product-inventory";
import {
  type BrainDeletionPreview,
  type BrainDeletionReceipt,
  deletionConfirmationReady,
  parseBrainDeletionPreview,
  parseBrainDeletionResult,
  previewFacts,
} from "@/lib/brain-deletion-view";

/// Delete an Organization Brain with the account's own hosted Brain key:
/// review the server's exact scope, type the Brain ID, then delete exactly
/// that previewed state. The Brain server decides admin standing; this panel
/// only shows what it answers.

type Phase =
  | { kind: "closed" }
  | { kind: "loading" }
  | { kind: "preview"; preview: BrainDeletionPreview; typed: string; error?: string; stale?: boolean }
  | { kind: "deleting"; preview: BrainDeletionPreview }
  | { kind: "deleted"; preview: BrainDeletionPreview; receipt: BrainDeletionReceipt }
  | { kind: "unavailable"; message: string };

async function readJson(response: Response) {
  const body = await response.json().catch(() => ({}));
  if (!response.ok) {
    const message = typeof body?.error === "string" ? body.error : `HTTP ${response.status}`;
    throw Object.assign(new Error(message), { status: response.status });
  }
  return body;
}

function failureStatus(error: unknown) {
  return typeof (error as { status?: unknown })?.status === "number" ? (error as { status: number }).status : 0;
}

export function BrainDeletionPanel({ brain, onDeleted }: { brain: BrainRow; onDeleted: () => void }) {
  const [phase, setPhase] = useState<Phase>({ kind: "closed" });

  async function review() {
    setPhase({ kind: "loading" });
    try {
      const response = await fetch(`/api/brain/deletion?brainId=${encodeURIComponent(brain.id)}`, { cache: "no-store" });
      setPhase({ kind: "preview", preview: parseBrainDeletionPreview(await readJson(response)), typed: "" });
    } catch (error) {
      const status = failureStatus(error);
      setPhase({
        kind: "unavailable",
        message: status === 403
          ? `Only a current admin of ${brain.name} can delete it, and your account isn't one.`
          : error instanceof Error ? error.message : "Brain deletion is unavailable right now.",
      });
    }
  }

  async function remove(preview: BrainDeletionPreview, typed: string) {
    setPhase({ kind: "deleting", preview });
    try {
      const response = await fetch("/api/brain/deletion", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ brainId: preview.brainId, stateDigest: preview.stateDigest, confirmation: typed }),
      });
      const { receipt } = parseBrainDeletionResult(await readJson(response));
      setPhase({ kind: "deleted", preview, receipt });
    } catch (error) {
      setPhase({
        kind: "preview",
        preview,
        typed,
        stale: failureStatus(error) === 409,
        error: error instanceof Error ? error.message : "The Brain wasn't deleted.",
      });
    }
  }

  if (brain.kind !== "Organization" || brain.pending) return null;
  return (
    <section aria-label={`Delete ${brain.name}`} className="mt-10 space-y-3 rounded-xl border border-destructive/30 p-4">
      <h2 className="text-base font-semibold">Delete Brain</h2>
      {phase.kind === "closed" && <>
        <p className="text-sm text-muted-foreground">
          A current admin can permanently delete {brain.name} for everyone. We&apos;ll check your account&apos;s access and show exactly what would be deleted.
        </p>
        <Button type="button" variant="destructive" onClick={() => void review()}>Review deletion…</Button>
      </>}
      {phase.kind === "loading" && <p className="flex items-center gap-2 text-sm"><Loader2Icon aria-hidden className="size-4 animate-spin" />Checking what would be deleted…</p>}
      {phase.kind === "unavailable" && <>
        <p role="alert" className="text-sm">{phase.message}</p>
        <Button type="button" variant="outline" onClick={() => setPhase({ kind: "closed" })}>Close</Button>
      </>}
      {(phase.kind === "preview" || phase.kind === "deleting") && <DeletionReview
        phase={phase}
        onType={typed => phase.kind === "preview" && !phase.stale && setPhase({ ...phase, typed, error: undefined })}
        onDelete={() => phase.kind === "preview" && void remove(phase.preview, phase.typed)}
        onReviewAgain={() => void review()}
        onCancel={() => setPhase({ kind: "closed" })}
      />}
      {phase.kind === "deleted" && <div role="status" className="space-y-1 text-sm">
        <p className="font-medium">Deleted {phase.preview.name} ({phase.receipt.brainId}).</p>
        <p className="text-muted-foreground">
          Deleted at <time dateTime={phase.receipt.deletedAt}>{phase.receipt.deletedAt}</time> · receipt{" "}
          <span className="font-mono text-xs" title={phase.receipt.deletionEventId}>{phase.receipt.deletionEventId.slice(0, 12)}…</span>
        </p>
        <p className="text-muted-foreground">Local copies and exports people already hold aren&apos;t erased.</p>
        <Button type="button" variant="outline" onClick={onDeleted}>Done</Button>
      </div>}
    </section>
  );
}

function DeletionReview({ phase, onType, onDelete, onReviewAgain, onCancel }: {
  phase: Extract<Phase, { kind: "preview" | "deleting" }>;
  onType: (typed: string) => void;
  onDelete: () => void;
  onReviewAgain: () => void;
  onCancel: () => void;
}) {
  const { preview } = phase;
  const typed = phase.kind === "preview" ? phase.typed : preview.brainId;
  const deleting = phase.kind === "deleting";
  // A refused stale preview must be reviewed again before anything is signed.
  const stale = phase.kind === "preview" && Boolean(phase.stale);
  return <div className="space-y-3 text-sm">
    <p className="font-medium">Delete {preview.name} permanently?</p>
    <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1">
      {previewFacts(preview).map(([label, value]) => <div key={label} className="contents">
        <dt className="text-muted-foreground">{label}</dt><dd className="break-all">{value}</dd>
      </div>)}
    </dl>
    {preview.retiredMountRecordCount > 0 && <p className="text-muted-foreground">
      {preview.retiredMountRecordCount} pending or revoked mount records that name this Brain are removed too.
    </p>}
    {!preview.deletable ? <div role="alert" className="space-y-1">
      <p className="flex items-center gap-2 font-medium"><TriangleAlertIcon aria-hidden className="size-4" />Revoke these active mounts first:</p>
      <ul className="list-disc pl-5">{preview.blockingMounts.map(mount => <li key={mount.connectionId}>
        {mount.direction === "outgoing" ? "Shared out" : "Mounted in"}: {mount.sourceBrainId}/{mount.sourceFolderId} → {mount.destinationBrainId}
        {" "}<span className="font-mono text-xs">({mount.connectionId})</span>
      </li>)}</ul>
    </div> : <>
      <p>
        This deletes the Brain, its folders, pages and history for everyone, and stops every invitation and
        invite token. It can&apos;t be undone here; only an operator restore from backup brings it back.
        Local copies and exports people already hold aren&apos;t erased.
      </p>
      <div className="space-y-1">
        <Label htmlFor={`delete-${preview.brainId}`}>Type <span className="font-mono">{preview.brainId}</span> to confirm</Label>
        <Input id={`delete-${preview.brainId}`} autoComplete="off" spellCheck={false} value={typed}
          disabled={deleting || stale} onChange={event => onType(event.target.value)} />
      </div>
    </>}
    {phase.kind === "preview" && phase.error && <p role="alert" className="text-destructive">{phase.error}</p>}
    <div className="flex flex-wrap gap-2">
      {stale
        ? <Button type="button" variant="outline" onClick={onReviewAgain}>Review the current Brain again</Button>
        : preview.deletable && <Button type="button" variant="destructive" disabled={deleting || !deletionConfirmationReady(preview, typed)} onClick={onDelete}>
          {deleting && <Loader2Icon aria-hidden className="size-4 animate-spin" />}Delete Brain permanently
        </Button>}
      <Button type="button" variant="ghost" disabled={deleting} onClick={onCancel}>Cancel</Button>
    </div>
  </div>;
}
