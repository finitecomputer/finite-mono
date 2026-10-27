"use client";
import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { useAgentInventory } from "@/hooks/use-agent-inventory";
import type { BrainIdentity } from "@/lib/brain-identity-roster";
import {
  RosterChangeError, changeMessage, inviteTarget, inviteToBrain, readPendingInvitations, removalMessage,
  revokeInvitation, shortNpub, startRemoval, waitForRemoval, type PendingInvitation,
} from "@/lib/brain-roster-changes";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";

export type ChangeNotice = { tone: "status" | "alert"; text: string };
type Action = "invite" | "revoke" | "remove";

/** One change at a time on this page. Leaving the page stops waiting for a
 * result; the change itself continues on the agent and the roster shows it. */
export function useRosterChangeRunner(agentName: string, onSettled: () => void) {
  const [progress, setProgress] = useState<string | null>(null);
  const [notice, setNotice] = useState<ChangeNotice | null>(null);
  const pending = useRef<AbortController | null>(null);
  useEffect(() => () => pending.current?.abort(), []);
  const run = useCallback(async (action: Action, who: string, working: string,
    change: (signal: AbortSignal) => Promise<ChangeNotice>) => {
    if (pending.current) return false;
    const controller = new AbortController();
    pending.current = controller;
    setProgress(working);
    setNotice(null);
    let ok = false;
    try {
      setNotice(await change(controller.signal));
      ok = true;
    } catch (error) {
      if (controller.signal.aborted) return false;
      const code = error instanceof RosterChangeError ? error.code : "unknown";
      setNotice({ tone: "alert", text: error instanceof RosterChangeError && code === "access" ? error.message
        : changeMessage(code, agentName, action, who) });
    } finally {
      if (!controller.signal.aborted) {
        pending.current = null;
        setProgress(null);
        onSettled();
      }
    }
    return ok;
  }, [agentName, onSettled]);
  return { progress, notice, busy: progress !== null, run };
}

export function ChangeNotices({ progress, notice }: { progress: string | null; notice: ChangeNotice | null }) {
  return <>
    <p role="status" aria-live="polite" className="text-sm text-muted-foreground">{progress ?? (notice?.tone === "status" ? notice.text : "")}</p>
    {!progress && notice?.tone === "alert" && <p role="alert" className="text-sm text-amber-700 dark:text-amber-400">{notice.text}</p>}
  </>;
}

/** Name, email or public name for one key; a key with none is "Unknown key". */
export function personLabel(row: BrainIdentity) {
  return (row.kind === "agent" ? row.name : row.name ?? row.email) ?? row.publicName ?? "Unknown key";
}

function invitationLabel(item: PendingInvitation) {
  if (item.targetName && item.targetNpub) return `${item.targetName} (${shortNpub(item.targetNpub)})`;
  return item.targetName ?? (item.targetNpub ? shortNpub(item.targetNpub) : "an invitation without a key");
}

type Runner = ReturnType<typeof useRosterChangeRunner>;

export function BrainInvitePanel({ runtimeId, brainId, brainName, agentName, runner, revision }: {
  runtimeId: string; brainId: string; brainName: string; agentName: string; runner: Runner; revision: number;
}) {
  const read = useCallback((id: string, signal: AbortSignal) => readPendingInvitations(id, brainId, signal), [brainId]);
  const invitations = useAgentInventory(runtimeId, read);
  const reload = invitations.refresh;
  // A settled change bumps the revision; the first load happens on mount.
  useEffect(() => { if (revision > 0) void reload(); }, [revision, reload]);
  const [value, setValue] = useState("");
  const [invalid, setInvalid] = useState(false);
  async function invite(event: FormEvent) {
    event.preventDefault();
    const target = inviteTarget(value);
    setInvalid(!target);
    if (!target) return;
    const sent = await runner.run("invite", target, `Inviting ${target}…`, async signal => {
      const created = await inviteToBrain(runtimeId, brainId, target, signal);
      const who = created?.targetNpub && created.targetNpub !== target ? `${target} (${shortNpub(created.targetNpub)})`
        : target.startsWith("npub1") ? shortNpub(target) : target;
      return { tone: "status", text: `Invited ${who} to ${brainName} as a Member. They accept it from their Agent.` };
    });
    if (sent) setValue("");
  }
  async function revoke(item: PendingInvitation) {
    const who = invitationLabel(item);
    await runner.run("revoke", who, `Revoking the invitation for ${who}…`, async signal => {
      await revokeInvitation(runtimeId, brainId, item.id, signal);
      return { tone: "status", text: `Revoked the invitation for ${who}.` };
    });
  }
  const rows = invitations.data;
  return <div className="space-y-4">
    <form className="space-y-2" onSubmit={event => void invite(event)} noValidate>
      <label htmlFor={`invite-${brainId}`} className="block text-sm font-medium">Invite someone</label>
      <div className="flex flex-wrap items-center gap-2">
        <Input id={`invite-${brainId}`} className="max-w-sm" value={value} autoComplete="off" spellCheck={false}
          placeholder="name@finite.vip or npub1…" aria-invalid={invalid || undefined}
          aria-describedby={`invite-${brainId}-hint`} maxLength={254}
          onChange={event => { setValue(event.target.value); setInvalid(false); }} />
        <Button type="submit" variant="outline" disabled={runner.busy || !value.trim()}>Invite</Button>
      </div>
      <p id={`invite-${brainId}-hint`} aria-live="polite" className="text-xs text-muted-foreground">
        {invalid ? "Enter an npub or a public name such as name@finite.vip."
          : `${agentName} signs a Member invitation. They join after they accept it from their Agent. No email is sent.`}
      </p>
    </form>
    <section aria-label={`Pending invitations for ${brainName}`} aria-busy={invitations.busy} className="space-y-2">
      <h3 className="text-sm font-medium">Pending invitations</h3>
      {!rows && <p className="text-sm text-muted-foreground">{invitations.busy ? "Loading invitations…" : "Pending invitations unavailable"}</p>}
      {rows?.length === 0 && <p className="text-sm text-muted-foreground">No pending invitations.</p>}
      {rows && rows.length > 0 && <ul className="divide-y rounded-lg border">
        {rows.map(item => <li key={item.id} className="flex flex-wrap items-center justify-between gap-2 px-3 py-2 text-sm">
          <span>
            <span className="font-medium">{item.targetName ?? (item.targetNpub ? "Key" : "No key yet")}</span>
            {item.targetNpub && <span className="ml-2 font-mono text-xs" title={item.targetNpub}>{shortNpub(item.targetNpub)}</span>}
            <span className="block text-xs text-muted-foreground">{item.expired ? "Expired" : `Expires ${item.expiresAt.slice(0, 10)}`}</span>
          </span>
          <Button type="button" variant="outline" size="sm" disabled={runner.busy}
            aria-label={`Revoke invitation for ${invitationLabel(item)}`} onClick={() => void revoke(item)}>Revoke</Button>
        </li>)}
      </ul>}
    </section>
  </div>;
}

/** The title and description name the key from Finite's records. Your
 * private alias is shown apart from them, beside the full key that is removed. */
export function RemoveIdentityDialog({ row, alias, brainName, agentName, onCancel, onConfirm }: {
  row: BrainIdentity | null; alias?: string | null; brainName: string; agentName: string;
  onCancel: () => void; onConfirm: (row: BrainIdentity) => void;
}) {
  const label = row ? personLabel(row) : "";
  const contact = !row ? null : row.kind !== "agent" && row.name && row.email ? row.email
    : row.publicName && row.publicName !== label ? row.publicName : null;
  return <Dialog open={row !== null} onOpenChange={open => { if (!open) onCancel(); }}>
    {row && <DialogContent>
      <DialogHeader>
        <DialogTitle>Remove {label}?</DialogTitle>
        <DialogDescription>
          {label}{contact ? `, ${contact}` : ""} (<span className="font-mono" title={row.npub}>{shortNpub(row.npub)}</span>) will lose access to {brainName}.
        </DialogDescription>
      </DialogHeader>
      <dl className="grid gap-2 rounded-lg bg-muted/50 p-3 text-sm">
        {alias && <div><dt className="text-xs text-muted-foreground">Your alias</dt><dd className="break-words">{alias}</dd></div>}
        <div><dt className="text-xs text-muted-foreground">Key</dt><dd className="font-mono text-xs break-all">{row.npub}</dd></div>
      </dl>
      <ul className="list-disc space-y-1 pl-5 text-sm">
        {row.role === "admin" && <li>{label} is an Admin. {agentName} removes the Admin role first, then removes them.</li>}
        <li>{agentName} re-keys the Folders they could read, so they cannot read anything added after this.</li>
        <li>Removal cannot take back what they already read or copied.</li>
        <li>This can take a few minutes for large Folders.</li>
      </ul>
      <DialogFooter>
        <DialogClose asChild><Button type="button" variant="outline">Cancel</Button></DialogClose>
        <Button type="button" variant="destructive" onClick={() => onConfirm(row)}>Remove</Button>
      </DialogFooter>
    </DialogContent>}
  </Dialog>;
}

/** Start one removal and wait for its background job on the agent. */
export function runRemoval(runner: Runner, runtimeId: string, brainId: string, agentName: string, row: BrainIdentity) {
  const who = `${personLabel(row)} (${shortNpub(row.npub)})`;
  return runner.run("remove", who, `Removing ${who}. This can take a few minutes.`, async signal => {
    const job = await startRemoval(runtimeId, brainId, row.npub, signal);
    const state = await waitForRemoval(runtimeId, brainId, job, signal);
    return { tone: state === "removed" ? "status" : "alert", text: removalMessage(state, agentName, who) };
  });
}
