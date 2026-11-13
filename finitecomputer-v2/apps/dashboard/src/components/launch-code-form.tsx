"use client";

import { useEffect, useState, type FormEvent } from "react";
import Link from "next/link";

const DRAFT_KEY = "finite-organization-launch-code";
type Draft = { launchCode: string; displayName: string; idempotencyKey: string };
const emptyDraft: Draft = { launchCode: "", displayName: "", idempotencyKey: "" };

export function LaunchCodeForm({ signedIn }: { signedIn: boolean }) {
  const [draft, setDraft] = useState(emptyDraft);
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [needsAuth, setNeedsAuth] = useState(!signedIn);

  useEffect(() => {
    let saved = emptyDraft;
    try {
      const value = JSON.parse(sessionStorage.getItem(DRAFT_KEY) ?? "null");
      if (value && typeof value.launchCode === "string" && typeof value.displayName === "string" &&
          typeof value.idempotencyKey === "string") saved = value;
    } catch { /* Storage may be unavailable; manual entry still works. */ }
    // Fragments never reach the server or the authentication provider.
    const fragment = new URLSearchParams(window.location.hash.slice(1));
    if (fragment.has("code")) {
      saved = { launchCode: fragment.get("code")!.slice(0, 256), displayName: (fragment.get("name") ?? "").slice(0, 80), idempotencyKey: saved.launchCode.trim() === fragment.get("code")!.trim() && saved.idempotencyKey ? saved.idempotencyKey : crypto.randomUUID() };
      window.history.replaceState(null, "", window.location.pathname);
    }
    if (!saved.idempotencyKey) saved = { ...saved, idempotencyKey: crypto.randomUUID() };
    // Hydrate this tab's draft only after mount; the server never sees the code.
    setDraft(saved);
    setReady(true);
    try { sessionStorage.setItem(DRAFT_KEY, JSON.stringify(saved)); } catch { /* Optional storage. */ }
  }, []);

  function update(field: "launchCode" | "displayName", value: string) {
    const next = { ...draft, [field]: value };
    setDraft(next);
    setError("");
    try { sessionStorage.setItem(DRAFT_KEY, JSON.stringify(next)); } catch { /* Optional storage. */ }
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy || needsAuth) return;
    setBusy(true);
    setError("");
    try {
      const response = await fetch("/code/redeem", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(draft),
      });
      const result = await response.json();
      if (response.status === 401) setNeedsAuth(true);
      if (!response.ok) throw new Error(result.error ?? "Could not create your agent. Please try again.");
      try { sessionStorage.removeItem(DRAFT_KEY); } catch { /* Optional storage. */ }
      window.location.assign(result.location);
    } catch (error) {
      setError(error instanceof Error ? error.message : "Could not create your agent. Please try again.");
      setBusy(false);
    }
  }

  return (
    <main className="flex min-h-dvh items-center justify-center bg-background px-6 py-12 text-foreground">
      <section className="w-full max-w-sm space-y-7">
        <Link href="/" className="text-sm text-muted-foreground hover:text-foreground">← Finite Computer</Link>
        <div className="space-y-3">
          <h1 className="text-3xl font-medium tracking-tight">Your organization launch code</h1>
          <p className="text-sm leading-6 text-muted-foreground">Enter the code your organization shared with you and choose a name for your agent. Your code includes your hosting tier.</p>
        </div>
        <form onSubmit={submit} className="space-y-5">
          <div className="space-y-2">
            <label htmlFor="launch-code" className="block text-sm font-medium">Launch code</label>
            <input id="launch-code" name="launchCode" value={draft.launchCode} onChange={event => update("launchCode", event.target.value)} required maxLength={256} autoComplete="off" autoCapitalize="none" spellCheck={false} disabled={!ready || busy} className="w-full rounded-lg border bg-background px-3 py-2 text-base" />
          </div>
          <div className="space-y-2">
            <label htmlFor="agent-name" className="block text-sm font-medium">Agent name</label>
            <input id="agent-name" name="displayName" value={draft.displayName} onChange={event => update("displayName", event.target.value)} required maxLength={80} autoComplete="off" disabled={!ready || busy} className="w-full rounded-lg border bg-background px-3 py-2 text-base" />
          </div>
          {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
          {needsAuth ? (
            <div className="space-y-3">
              <p className="text-sm text-muted-foreground">Sign up or log in to continue. You’ll review and submit your code after signing in.</p>
              <div className="flex gap-3">
                <a href="/signup?returnTo=/code" className="rounded-lg bg-primary px-4 py-2 text-sm text-primary-foreground">Sign up</a>
                <a href="/login?returnTo=/code" className="rounded-lg border px-4 py-2 text-sm">Log in</a>
              </div>
            </div>
          ) : (
            <button type="submit" disabled={!ready || busy || !draft.launchCode.trim() || !draft.displayName.trim()} className="w-full rounded-lg bg-primary px-4 py-3 text-sm font-medium text-primary-foreground disabled:opacity-50">{busy ? "Creating your agent…" : "Create agent"}</button>
          )}
          <p className="text-xs leading-5 text-muted-foreground">Opening this page does not use your code. It is used only when you select Create agent.</p>
        </form>
      </section>
    </main>
  );
}
