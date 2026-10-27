/**
 * Private key aliases: a label the signed-in account gives one exact key.
 * Only that account sees it. It is never identity evidence and never changes
 * Brain access. Core is the authority for validation; this module mirrors its
 * rules so the editor can explain a problem before saving.
 */

export type KeyAlias = { npub: string; alias: string };

export const MAX_ALIAS_CHARS = 64;
const MAX_ALIAS_BYTES = 256;
/** Keys per lookup; the roster lists at most this many identities. */
export const MAX_ALIAS_LOOKUP_KEYS = 1024;
const NPUB = /^npub1[023456789acdefghjklmnpqrstuvwxyz]{58}$/;
// Control characters and bidirectional overrides can hide or reorder text.
// eslint-disable-next-line no-control-regex
const UNSAFE = /[\u0000-\u001f\u007f-\u009f\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/;

export class KeyAliasError extends Error {
  constructor(message: string, readonly status: number) { super(message); }
}

export function isNpub(value: unknown): value is string {
  return typeof value === "string" && NPUB.test(value);
}

/** Trimmed alias with inner whitespace collapsed, or the reason it is invalid. */
export function normalizeAlias(value: string): { alias: string } | { error: string } {
  if (UNSAFE.test(value)) return { error: "Use letters, numbers, spaces and punctuation only." };
  const alias = value.trim().split(/\s+/).filter(Boolean).join(" ");
  if (!alias) return { error: "Enter an alias, or clear it instead." };
  if ([...alias].length > MAX_ALIAS_CHARS || new TextEncoder().encode(alias).length > MAX_ALIAS_BYTES) {
    return { error: `Use ${MAX_ALIAS_CHARS} characters or fewer.` };
  }
  return { alias };
}

export function aliasLength(value: string) {
  return [...value.trim().split(/\s+/).filter(Boolean).join(" ")].length;
}

/** One alias from Core or the dashboard route; anything else is refused. */
export function parseKeyAlias(value: unknown): KeyAlias {
  const row = value && typeof value === "object" ? value as Record<string, unknown> : {};
  if (!isNpub(row.npub) || typeof row.alias !== "string" || !row.alias || [...row.alias].length > MAX_ALIAS_CHARS) {
    throw new KeyAliasError("Aliases returned an unexpected response.", 502);
  }
  return { npub: row.npub, alias: row.alias };
}

export function parseKeyAliases(value: unknown): KeyAlias[] {
  const list = value && typeof value === "object" ? (value as Record<string, unknown>).aliases : null;
  if (!Array.isArray(list) || list.length > MAX_ALIAS_LOOKUP_KEYS) {
    throw new KeyAliasError("Aliases returned an unexpected response.", 502);
  }
  return list.map(parseKeyAlias);
}

async function request(init: RequestInit & { body: string }) {
  const response = await fetch("/api/brain/key-aliases", {
    ...init, cache: "no-store", headers: { "content-type": "application/json" },
  });
  const payload = response.status === 204 ? null : await response.json().catch(() => null);
  if (!response.ok) {
    const message = payload && typeof payload.error === "string" ? payload.error : "Aliases are unavailable. Try again.";
    throw new KeyAliasError(message, response.status);
  }
  return payload;
}

/** Your aliases for these keys, keyed by npub. */
export async function readKeyAliases(npubs: string[], signal: AbortSignal) {
  const aliases = parseKeyAliases(await request({ method: "POST", body: JSON.stringify({ npubs }), signal }));
  return new Map(aliases.map(row => [row.npub, row.alias]));
}

export async function saveKeyAlias(npub: string, alias: string, signal?: AbortSignal) {
  return parseKeyAlias(await request({ method: "PUT", body: JSON.stringify({ npub, alias }), signal }));
}

export async function clearKeyAlias(npub: string, signal?: AbortSignal) {
  await request({ method: "DELETE", body: JSON.stringify({ npub }), signal });
}
