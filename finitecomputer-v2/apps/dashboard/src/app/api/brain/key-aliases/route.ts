import { clearCoreKeyAlias, CoreFetchError, lookupCoreKeyAliases, setCoreKeyAlias } from "@/lib/core-client";
import { isNpub, MAX_ALIAS_LOOKUP_KEYS, normalizeAlias } from "@/lib/brain-key-aliases";
import { requestOriginMatchesHost } from "@/lib/http-headers";

// Private key aliases for the signed-in account. The browser sends only keys
// and alias text; Core derives the account from the server-held session.
export const POST = handle;
export const PUT = handle;
export const DELETE = handle;

const headers = { "cache-control": "no-store" };
const fail = (status: number, error = "Aliases are unavailable right now. Try again.") =>
  Response.json({ error }, { status, headers });

async function handle(request: Request) {
  if (!requestOriginMatchesHost(request)) return fail(403);
  const payload = await request.json().catch(() => null) as Record<string, unknown> | null;
  if (!payload || typeof payload !== "object") return fail(400, "That request was not understood.");
  try {
    if (request.method === "POST") {
      const npubs = payload.npubs;
      if (!Array.isArray(npubs) || npubs.length > MAX_ALIAS_LOOKUP_KEYS || !npubs.every(isNpub)) {
        return fail(400, "That request was not understood.");
      }
      const unique = [...new Set(npubs)];
      const aliases = unique.length ? await lookupCoreKeyAliases(unique, request.signal) : [];
      return Response.json({ aliases }, { headers });
    }
    if (!isNpub(payload.npub)) return fail(400, "That key is not valid.");
    if (request.method === "DELETE") {
      await clearCoreKeyAlias(payload.npub, request.signal);
      return new Response(null, { status: 204, headers });
    }
    const normalized = typeof payload.alias === "string" ? normalizeAlias(payload.alias) : null;
    if (!normalized || "error" in normalized) return fail(400, normalized?.error ?? "Enter an alias.");
    return Response.json(await setCoreKeyAlias(payload.npub, normalized.alias, request.signal), { headers });
  } catch (error) {
    if (!(error instanceof CoreFetchError)) return fail(503);
    if (error.status === 400) return fail(400, "That alias can't be saved. Use 64 characters or fewer, without control characters.");
    if (error.status === 409) return fail(409, "You've reached the alias limit. Clear an alias you no longer need.");
    if ([401, 403].includes(error.status)) return fail(error.status, "Sign in again to manage aliases.");
    // An older Core without alias routes answers 404 or 405.
    if ([404, 405].includes(error.status)) return fail(404);
    return fail(503);
  }
}
