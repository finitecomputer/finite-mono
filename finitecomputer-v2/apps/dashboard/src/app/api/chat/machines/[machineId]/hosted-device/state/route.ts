import {
  bootstrapHostedWebChat,
  HostedWebChatError,
} from "@/lib/hosted-web-chat";

export const dynamic = "force-dynamic";

export async function GET(
  _request: Request,
  { params }: { params: Promise<{ machineId: string }> }
) {
  try {
    const { machineId } = await params;
    return Response.json(await bootstrapHostedWebChat(machineId), {
      headers: { "cache-control": "no-store" },
    });
  } catch (error) {
    return chatErrorResponse(error);
  }
}

function chatErrorResponse(error: unknown) {
  const status = error instanceof HostedWebChatError ? error.status : 502;
  const message = error instanceof Error ? error.message : "Hosted web chat is unavailable.";
  return Response.json({ error: message }, { status });
}
