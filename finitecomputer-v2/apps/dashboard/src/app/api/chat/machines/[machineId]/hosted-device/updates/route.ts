import {
  HostedWebChatError,
  hostedWebChatErrorMessage,
  streamHostedWebChat,
} from "@/lib/hosted-web-chat";
import { CHAT_UNAVAILABLE_MESSAGE } from "@/lib/chat-product-copy";

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
        { error: CHAT_UNAVAILABLE_MESSAGE },
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
    if (!(error instanceof HostedWebChatError)) {
      console.warn("Hosted web chat update stream failed", {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return Response.json({ error: hostedWebChatErrorMessage(error) }, { status });
  }
}
