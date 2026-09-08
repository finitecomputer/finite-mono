import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { gateReturnPath, gateSiteOrigin, mintSiteGateSession, SiteGateError } from "@/lib/site-gate";

export const dynamic = "force-dynamic";

export async function GET(request: Request) {
  const headers = { "cache-control": "no-store", "referrer-policy": "no-referrer" };
  try {
    const query = new URL(request.url).searchParams;
    const output = gateSiteOrigin(query.get("output") ?? "");
    const returnTo = gateReturnPath(query.get("return_to") ?? "/");
    const account = await getAccountAuthContext();
    if (!account.workosUserId) {
      const continuation = `/site-auth?${new URLSearchParams({ output, return_to: returnTo })}`;
      return new Response(null, { status: 303, headers: { ...headers, location: `/login?${new URLSearchParams({ returnTo: continuation })}` } });
    }
    const location = await mintSiteGateSession(account, output, returnTo);
    return new Response(null, { status: 303, headers: { ...headers, location } });
  } catch (error) {
    return Response.json({ error: error instanceof SiteGateError ? error.message : "Site sign-in is temporarily unavailable." },
      { status: error instanceof SiteGateError ? error.status : 503, headers });
  }
}
