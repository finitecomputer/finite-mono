import {
  dispatchHostedWebChatAction,
  HostedWebChatError,
  hostedWebChatErrorMessage,
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
    if (!(error instanceof HostedWebChatError)) {
      console.warn("Hosted web chat action failed", {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return Response.json({ error: hostedWebChatErrorMessage(error) }, { status });
  }
}
