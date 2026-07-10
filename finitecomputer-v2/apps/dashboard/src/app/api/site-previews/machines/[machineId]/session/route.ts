import {
  createSitePreviewSession,
  readBoundedSitePreviewUrl,
  SitePreviewError,
} from "@/lib/site-preview";

export const dynamic = "force-dynamic";

export async function POST(
  request: Request,
  { params }: { params: Promise<{ machineId: string }> }
) {
  try {
    const url = await readBoundedSitePreviewUrl(request);
    const { machineId } = await params;
    return Response.json(await createSitePreviewSession(machineId, url), {
      headers: { "cache-control": "no-store" },
    });
  } catch (error) {
    const status = error instanceof SitePreviewError ? error.status : 400;
    const message = error instanceof SitePreviewError
      ? error.message
      : "Choose a Finite site to preview.";
    return Response.json({ error: message }, {
      status,
      headers: { "cache-control": "no-store" },
    });
  }
}
