import {
  HostedWebChatError,
  streamHostedWebChatAttachment,
} from "@/lib/hosted-web-chat";

export const dynamic = "force-dynamic";

export async function GET(
  request: Request,
  {
    params,
  }: {
    params: Promise<{
      machineId: string;
      roomId: string;
      messageId: string;
      attachmentId: string;
    }>;
  }
) {
  try {
    const { machineId, roomId, messageId, attachmentId } = await params;
    const upstream = await streamHostedWebChatAttachment(
      machineId,
      roomId,
      messageId,
      attachmentId,
      request.signal
    );
    if (!upstream.ok || !upstream.body) {
      const message = await upstream.text();
      return Response.json(
        { error: message.slice(0, 500) || "Attachment is unavailable." },
        { status: upstream.status || 502 }
      );
    }

    const headers = new Headers({ "cache-control": "private, no-store" });
    for (const name of [
      "content-type",
      "content-disposition",
      "content-length",
      "content-security-policy",
      "x-content-type-options",
    ]) {
      const value = upstream.headers.get(name);
      if (value) {
        headers.set(name, value);
      }
    }
    return new Response(upstream.body, { status: 200, headers });
  } catch (error) {
    const status = error instanceof HostedWebChatError ? error.status : 502;
    const message = error instanceof Error ? error.message : "Attachment is unavailable.";
    return Response.json({ error: message }, { status });
  }
}
