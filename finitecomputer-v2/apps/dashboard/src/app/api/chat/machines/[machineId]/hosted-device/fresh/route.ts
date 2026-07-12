import {
  HostedWebChatError,
  hostedWebChatErrorMessage,
  startFreshHostedWebChat,
} from "@/lib/hosted-web-chat";

export async function POST(
  _request: Request,
  { params }: { params: Promise<{ machineId: string }> }
) {
  try {
    const { machineId } = await params;
    return Response.json(await startFreshHostedWebChat(machineId), {
      headers: { "cache-control": "no-store" },
    });
  } catch (error) {
    const status = error instanceof HostedWebChatError ? error.status : 502;
    if (!(error instanceof HostedWebChatError)) {
      console.warn("Hosted web chat fresh-room recovery failed", {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return Response.json({ error: hostedWebChatErrorMessage(error) }, { status });
  }
}
