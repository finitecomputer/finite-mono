import {
  dispatchHostedWebChatAction,
  HostedWebChatError,
} from "@/lib/hosted-web-chat";

export async function POST(
  request: Request,
  { params }: { params: Promise<{ machineId: string }> }
) {
  try {
    const { machineId } = await params;
    return Response.json(
      await dispatchHostedWebChatAction(machineId, await request.json()),
      { headers: { "cache-control": "no-store" } }
    );
  } catch (error) {
    const status = error instanceof HostedWebChatError ? error.status : 502;
    const message = error instanceof Error ? error.message : "Hosted web chat is unavailable.";
    return Response.json({ error: message }, { status });
  }
}
