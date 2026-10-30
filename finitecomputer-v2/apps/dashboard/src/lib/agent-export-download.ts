import { EXPORT_FILE_NAME } from "@/lib/agent-export";
import { CoreFetchError, createCoreHostedHermesSession } from "@/lib/core-client";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import type { HostedHermesSession } from "@/lib/hosted-hermes-status";

/** Streams the selected Agent's finished export to the signed-in owner.
 *
 * A browser download cannot carry the native Hermes bearer, so the dashboard
 * gets an operation-local grant from Core, fetches the plugin's file and
 * passes the bytes through without buffering. Only a single byte range is
 * forwarded, so an interrupted download resumes where it stopped. */

type Dependencies = {
  access: (runtimeId: string) => Promise<{ machineId: string } | null>;
  grant: (runtimeId: string, signal: AbortSignal) => Promise<HostedHermesSession>;
  fetch: (input: URL, init: RequestInit) => Promise<Response>;
};
const server: Dependencies = {
  access: runtimeId => loadDashboardMachineAccess(runtimeId),
  grant: createCoreHostedHermesSession,
  fetch: (input, init) => fetch(input, init),
};
const RANGE = /^bytes=\d*-\d*$/;
const PASSED = ["content-length", "content-range", "accept-ranges", "etag", "last-modified"];

export async function proxyAgentExportDownload(request: Request, runtimeId: string, dependencies = server): Promise<Response> {
  const headers = { "cache-control": "private, no-store" };
  const fail = (status: number, error: string) => Response.json({ error }, { status, headers });
  // A download starts from the dashboard itself, never from another site's page.
  const site = request.headers.get("sec-fetch-site");
  if (site && site !== "same-origin" && site !== "none") return fail(403, "Start the download from the dashboard.");
  const access = await dependencies.access(runtimeId).catch(() => null);
  if (!access || access.machineId !== runtimeId) return fail(404, "This agent is unavailable.");

  const forwarded: Record<string, string> = {};
  const range = request.headers.get("range");
  if (range && RANGE.test(range)) {
    forwarded.range = range;
    const ifRange = request.headers.get("if-range");
    if (ifRange && ifRange.length <= 256) forwarded["if-range"] = ifRange;
  }
  for (let attempt = 0; attempt < 2; attempt++) {
    let grant: HostedHermesSession;
    try {
      grant = await dependencies.grant(runtimeId, request.signal);
    } catch (error) {
      if (error instanceof CoreFetchError && [401, 403, 404].includes(error.status)) {
        return fail(403, "Access to this agent is no longer available.");
      }
      if (error instanceof CoreFetchError && error.status === 409) {
        return fail(409, "This agent’s dashboard connection is not ready yet. Try again shortly.");
      }
      return fail(503, "The agent is unavailable. Try again.");
    }
    let upstream: Response;
    try {
      upstream = await dependencies.fetch(new URL("api/plugins/finite-export/download", grant.baseUrl), {
        headers: { ...forwarded, authorization: `Bearer ${grant.accessToken}` },
        redirect: "error", cache: "no-store", signal: request.signal,
      });
    } catch {
      return fail(502, "Could not reach the agent. Try again.");
    }
    // A fresh grant can race the agent's restart; reauthorize once.
    if (upstream.status === 401 && attempt === 0) {
      await upstream.body?.cancel();
      continue;
    }
    if ((upstream.status === 200 || upstream.status === 206) && upstream.body
      && upstream.headers.get("content-type") === "application/zip") {
      const named = /^attachment; filename="([^"]{1,255})"$/.exec(upstream.headers.get("content-disposition") ?? "")?.[1];
      const fileName = named && EXPORT_FILE_NAME.test(named) ? named : "finite-export.zip";
      const out = new Headers({
        ...headers,
        "content-type": "application/zip",
        "content-disposition": `attachment; filename="${fileName}"`,
        "x-content-type-options": "nosniff",
      });
      for (const name of PASSED) {
        const value = upstream.headers.get(name);
        if (value) out.set(name, value);
      }
      return new Response(upstream.body, { status: upstream.status, headers: out });
    }
    await upstream.body?.cancel();
    if (upstream.status === 416) {
      const total = upstream.headers.get("content-range");
      return new Response(null, { status: 416, headers: total ? { ...headers, "content-range": total } : headers });
    }
    if (upstream.status === 404) return fail(404, "No export is ready for this agent.");
    if (upstream.status === 401 || upstream.status === 403) return fail(403, "Access to this agent is no longer available.");
    return fail(502, "The agent couldn’t send the export. Try again.");
  }
  return fail(403, "Access to this agent is no longer available.");
}
