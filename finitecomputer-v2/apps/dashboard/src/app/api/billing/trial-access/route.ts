import { loadCoreBillingOverview } from "@/lib/core-client";

export async function GET() {
  const billing = await loadCoreBillingOverview({ cacheMode: "fresh" });
  if (!billing.billing) return Response.json({ error: "Billing is unavailable." }, { status: 503 });
  return Response.json({ blocked: billing.billing.trial_access?.blocked ?? false }, {
    headers: { "cache-control": "no-store, private" },
  });
}
