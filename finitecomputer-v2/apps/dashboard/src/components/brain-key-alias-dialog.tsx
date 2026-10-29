"use client";
import { useId, useState, type FormEvent } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { aliasLength, MAX_ALIAS_CHARS, normalizeAlias } from "@/lib/brain-key-aliases";

export type AliasTarget = { npub: string; alias: string | null; actual: string | null };

/** Add, edit or clear the signed-in account's private alias for one key. */
export function KeyAliasDialog({ target, otherAliases, onSave, onClear, onClose, onRestoreFocus }: {
  target: AliasTarget;
  /** Your aliases for the other keys in this list, to flag look-alikes. */
  otherAliases: string[];
  onSave: (npub: string, alias: string) => Promise<void>;
  onClear: (npub: string) => Promise<void>;
  onClose: () => void;
  onRestoreFocus: () => void;
}) {
  const inputId = useId();
  const hintId = useId();
  const [value, setValue] = useState(target.alias ?? "");
  const [busy, setBusy] = useState<"save" | "clear" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const normalized = normalizeAlias(value);
  const alias = "alias" in normalized ? normalized.alias : null;
  const problem = error ?? (value.trim() && "error" in normalized ? normalized.error : null);
  const duplicate = alias !== null && otherAliases.some(other => other.toLocaleLowerCase() === alias.toLocaleLowerCase());
  const run = async (kind: "save" | "clear", action: () => Promise<void>) => {
    setBusy(kind); setError(null);
    try { await action(); onClose(); } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Aliases are unavailable right now. Try again.");
    } finally { setBusy(null); }
  };
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!alias || busy) return;
    void run("save", () => onSave(target.npub, alias));
  };
  return <Dialog open onOpenChange={open => { if (!open && !busy) onClose(); }}>
    <DialogContent className="sm:max-w-md" onCloseAutoFocus={event => { event.preventDefault(); onRestoreFocus(); }}>
      <form onSubmit={submit} className="grid gap-4" aria-busy={busy !== null}>
        <DialogHeader>
          <DialogTitle>{target.alias ? "Edit your alias" : "Add your alias"}</DialogTitle>
          <DialogDescription>Only you can see this alias. It doesn’t change who this is or what they can access.</DialogDescription>
        </DialogHeader>
        <dl className="grid gap-2 rounded-lg bg-muted/50 p-3 text-sm">
          <div><dt className="text-xs text-muted-foreground">Finite shows</dt><dd className="break-words">{target.actual ?? "Unknown"}</dd></div>
          <div><dt className="text-xs text-muted-foreground">Key</dt><dd className="font-mono text-xs break-all">{target.npub}</dd></div>
        </dl>
        <div className="grid gap-1.5">
          <label htmlFor={inputId} className="text-sm font-medium">Alias</label>
          <Input id={inputId} value={value} autoFocus autoComplete="off" spellCheck={false} disabled={busy !== null}
            aria-describedby={hintId} aria-invalid={problem ? true : undefined}
            onChange={event => { setValue(event.target.value); setError(null); }} />
          <div id={hintId} className="flex justify-between gap-3 text-xs text-muted-foreground">
            <span>{duplicate ? "You already use this alias for another key here. Check the key before saving." : "A private reminder, like a nickname or role."}</span>
            <span className="tabular-nums">{aliasLength(value)}/{MAX_ALIAS_CHARS}<span className="sr-only"> characters</span></span>
          </div>
          {problem && <p role="alert" className="text-sm text-destructive">{problem}</p>}
        </div>
        <DialogFooter className="gap-2 sm:justify-between">
          {target.alias
            ? <Button type="button" variant="destructive" disabled={busy !== null}
                onClick={() => void run("clear", () => onClear(target.npub))}>
                {busy === "clear" ? "Clearing…" : "Clear alias"}
              </Button>
            : <span />}
          <div className="flex gap-2">
            <Button type="button" variant="outline" disabled={busy !== null} onClick={onClose}>Cancel</Button>
            <Button type="submit" disabled={busy !== null || alias === null || alias === target.alias}>
              {busy === "save" ? "Saving…" : "Save alias"}
            </Button>
          </div>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>;
}
