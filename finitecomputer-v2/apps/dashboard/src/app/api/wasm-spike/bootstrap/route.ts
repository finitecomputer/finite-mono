// Disposable local stand-in for Core returning the signed-in user's nsec.
// There is deliberately no chat action, decryption, or hosted-device call here.
export async function POST(request: Request) {
  if (process.env.FINITECHAT_WASM_SPIKE !== "1" || process.env.NODE_ENV === "production") {
    return new Response(null, { status: 404 });
  }
  const origin = request.headers.get("origin");
  if (!origin || new URL(origin).host !== request.headers.get("host")
      || !["localhost", "127.0.0.1"].includes(new URL(origin).hostname)) {
    return new Response(null, { status: 403 });
  }
  const port = process.env.FINITECHAT_WASM_SPIKE_PORT || "28789";
  if (!/^\d{1,5}$/.test(port)) return new Response(null, { status: 503 });
  const response = await fetch(`http://127.0.0.1:${port}/spike/bootstrap`, { cache: "no-store" });
  if (!response.ok) return new Response(null, { status: 503 });
  return Response.json(await response.json(), { headers: { "Cache-Control": "no-store" } });
}
