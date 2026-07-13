import {
  bootstrapHostedWebChat,
  HostedWebChatError,
  hostedWebChatErrorMessage,
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
  if (!(error instanceof HostedWebChatError)) {
    console.warn("Hosted web chat state failed", {
      error: error instanceof Error ? error.message : String(error),
    });
  }
  return Response.json(
    {
      error: hostedWebChatErrorMessage(error),
      ...(error instanceof HostedWebChatError && error.code
        ? { code: error.code }
        : {}),
    },
    { status }
  );
}
