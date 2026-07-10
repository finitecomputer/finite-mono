import {
  HostedWebChatError,
  streamHostedWebChat,
} from "@/lib/hosted-web-chat";

export const dynamic = "force-dynamic";

export async function GET(
  request: Request,
  { params }: { params: Promise<{ machineId: string }> }
) {
  try {
    const { machineId } = await params;
    const upstream = await streamHostedWebChat(machineId, request.signal);
    if (!upstream.ok || !upstream.body) {
      return Response.json(
        { error: "Chat updates are unavailable right now." },
        { status: upstream.status || 502 }
      );
    }
    return new Response(upstream.body, {
      status: 200,
      headers: {
        "cache-control": "no-cache, no-transform",
        connection: "keep-alive",
        "content-type": "text/event-stream",
        "x-accel-buffering": "no",
      },
    });
  } catch (error) {
    const status = error instanceof HostedWebChatError ? error.status : 502;
    const message = error instanceof Error ? error.message : "Hosted web chat is unavailable.";
    return Response.json({ error: message }, { status });
  }
}
