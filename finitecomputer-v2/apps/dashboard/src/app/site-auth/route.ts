import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { createSiteAccountSession, parseSitePreviewTarget, siteEmailSignInUrl, SitePreviewError } from "@/lib/site-preview";

export const dynamic = "force-dynamic";

export async function GET(request: Request) {
  const headers = { "cache-control": "no-store", "referrer-policy": "no-referrer" };
  try {
    const target = parseSitePreviewTarget(new URL(request.url).searchParams.get("url"));
    let location = siteEmailSignInUrl(target);
    try {
      const session = await createSiteAccountSession(target, await getAccountAuthContext());
      location = session.url;
    } catch {
      // Anonymous, unverified, and unavailable account sessions retain Sites'
      // existing email challenge. The explicit fallback cannot redirect here.
    }
    return new Response(null, { status: 303, headers: { ...headers, location } });
  } catch (error) {
    return Response.json({ error: "Choose a valid site." },
      { status: error instanceof SitePreviewError ? error.status : 400, headers });
  }
}
