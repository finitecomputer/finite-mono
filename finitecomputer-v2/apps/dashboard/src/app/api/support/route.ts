import { workosBaseUrl } from "@/lib/workos-auth";
import { CoreFetchError, submitCoreSupportReport } from "@/lib/core-client";

export async function POST(request: Request) {
  const headers = { "cache-control": "no-store" };
  // A JSON-only same-origin browser mutation; the server forwards only its session token.
  const origin = process.env.FC_DASHBOARD_BASE_URL?.trim() || workosBaseUrl() || request.url;
  if (request.headers.get("origin") !== new URL(origin).origin
      || request.headers.get("content-type")?.split(";")[0] !== "application/json") {
    return Response.json({ error: "Invalid support request origin." }, { status: 403, headers });
  }
  try {
    // Bound the streamed body, including clients without Content-Length.
    const reader = request.body?.getReader();
    if (!reader) return Response.json({ error: "A report is required." }, { status: 400, headers });
    const chunks: Uint8Array[] = [];
    let size = 0;
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) break;
      size += chunk.value.byteLength;
      if (size > 20 * 1024) {
        await reader.cancel();
        return Response.json({ error: "Report is too large." }, { status: 413, headers });
      }
      chunks.push(chunk.value);
    }
    const input: unknown = JSON.parse(Buffer.concat(chunks).toString("utf8"));
    return Response.json(await submitCoreSupportReport(input), { status: 202, headers });
  } catch (error) {
    const status = error instanceof CoreFetchError ? error.status : error instanceof SyntaxError ? 400 : 503;
    const message = error instanceof CoreFetchError && error.status < 500 ? error.message : "Could not confirm receipt. Retry this report or email support directly.";
    return Response.json({ error: message }, { status, headers });
  }
}
