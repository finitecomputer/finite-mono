import { loadCoreMe } from "@/lib/core-client";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";
import { movedRedirect } from "@/lib/finite-chat-move";

/** FIN-463: where this person goes now, for an open dashboard's recheck on
 * each navigation; the layout decides only on a full load. */
export async function GET() {
  const [viewer, core] = await Promise.all([loadOptionalViewerContext(), loadCoreMe({ cacheMode: "fresh" })]);
  return Response.json(
    { redirect: movedRedirect(core.me, viewer) },
    { headers: { "cache-control": "no-store, private" } },
  );
}
