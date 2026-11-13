"use client";

import { useCallback, useEffect, useState } from "react";
import { clearKeyAlias, readKeyAliases, saveKeyAlias } from "@/lib/brain-key-aliases";

type Loaded = { key: string; aliases: Map<string, string> | null };

/**
 * The signed-in account's private aliases for one roster's keys. A failed or
 * unsupported lookup leaves `status` "unavailable" and never hides the roster.
 * The caller remounts this hook per account, agent and Brain, so nothing
 * carries over between them. Results for an older key set are ignored.
 */
export function useKeyAliases(npubs: string[] | null) {
  const key = npubs?.join(",") ?? "";
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  // Your own saves since the lookup started. They win over a lookup that may
  // have been read before them.
  const [edits, setEdits] = useState<Map<string, string | null>>(() => new Map());
  useEffect(() => {
    if (!key) return;
    const controller = new AbortController();
    readKeyAliases(key.split(","), controller.signal).then(
      aliases => { if (!controller.signal.aborted) setLoaded({ key, aliases }); },
      () => { if (!controller.signal.aborted) setLoaded({ key, aliases: null }); },
    );
    return () => controller.abort();
  }, [key]);
  const current = loaded?.key === key ? loaded : null;
  const status = !key ? "ready" : !current ? "loading" : current.aliases ? "ready" : "unavailable";
  const aliasFor = useCallback((npub: string) => {
    if (edits.has(npub)) return edits.get(npub) ?? null;
    return current?.aliases?.get(npub) ?? null;
  }, [current, edits]);
  const save = useCallback(async (npub: string, alias: string) => {
    const saved = await saveKeyAlias(npub, alias);
    setEdits(previous => new Map(previous).set(npub, saved.alias));
  }, []);
  const clear = useCallback(async (npub: string) => {
    await clearKeyAlias(npub);
    setEdits(previous => new Map(previous).set(npub, null));
  }, []);
  return { status, aliasFor, save, clear } as const;
}
