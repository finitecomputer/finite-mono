import { CoreFetchError, createCoreHostedHermesSession } from "@/lib/core-client";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

/** Browser media elements cannot set a bearer header. Reauthorize each read;
 * no native token or agent URL is embedded in an image/download link. */
export async function GET(request: Request, context: { params: Promise<{ runtimeId: string }> }) {
  const headers = {
    "cache-control": "no-store",
    "x-content-type-options": "nosniff",
    "content-security-policy": "sandbox; default-src 'none'",
  };
  const fail = (status: number) => Response.json({ error: "Attachment is unavailable." }, { status, headers });
  const path = new URL(request.url).searchParams.get("path");
  if (!path || path.length > 4096 || /[\x00-\x1f]/.test(path)) return fail(400);
  try {
    const { runtimeId } = await context.params;
    const access = await loadDashboardMachineAccess(runtimeId, { coreCacheMode: "fresh" });
    if (!access || access.machineId !== runtimeId) return fail(404);
    const signal = AbortSignal.any([request.signal, AbortSignal.timeout(60_000)]);
    const grant = await createCoreHostedHermesSession(runtimeId, signal);
    const url = new URL("api/files/download", grant.baseUrl);
    url.searchParams.set("path", path);
    const response = await fetch(url, {
      headers: { authorization: `Bearer ${grant.accessToken}`, ...(request.headers.has("range") ? { range: request.headers.get("range")! } : {}) },
      cache: "no-store", redirect: "error", signal,
    });
    if (!response.ok) { await response.body?.cancel(); return fail(response.status === 404 ? 404 : 503); }
    const outgoing = new Headers(headers);
    // Always download documents on navigation; uploaded HTML must never run on
    // the dashboard origin. Image/audio/video elements can still display bytes.
    const filename = encodeURIComponent(path.split("/").at(-1) || "attachment").replace(/['()*]/g, char => `%${char.charCodeAt(0).toString(16)}`);
    outgoing.set("content-disposition", `attachment; filename*=UTF-8''${filename}`);
    for (const name of ["content-type", "content-length", "content-range", "accept-ranges"]) {
      const value = response.headers.get(name);
      if (value) outgoing.set(name, value);
    }
    return new Response(response.body, { status: response.status, headers: outgoing });
  } catch (error) {
    return fail(error instanceof CoreFetchError && [401, 403, 404].includes(error.status) ? error.status : 503);
  }
}
