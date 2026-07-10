import {
  HostedWebChatError,
  uploadHostedWebChatAttachments,
} from "@/lib/hosted-web-chat";

export const dynamic = "force-dynamic";

export async function POST(
  request: Request,
  { params }: { params: Promise<{ machineId: string }> }
) {
  try {
    const { machineId } = await params;
    return Response.json(
      await uploadHostedWebChatAttachments(machineId, await request.formData()),
      { headers: { "cache-control": "no-store" } }
    );
  } catch (error) {
    const status = error instanceof HostedWebChatError ? error.status : 502;
    const message = error instanceof Error ? error.message : "Attachment upload is unavailable.";
    return Response.json({ error: message }, { status });
  }
}
