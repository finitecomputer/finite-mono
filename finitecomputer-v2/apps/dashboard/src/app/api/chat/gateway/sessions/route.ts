import { HermesGatewayError, hermesSessionsTopic } from "@/lib/hermes-gateway";

export const dynamic = "force-dynamic";

/**
 * Spike: the hermes half of the side-by-side parity probe. Same response
 * shape the hosted path serves, sourced from tui_gateway session.list over
 * /api/ws instead of the finitechat hosted device.
 */
export async function GET() {
  try {
    return Response.json({ topic: await hermesSessionsTopic() }, {
      headers: { "cache-control": "no-store" },
    });
  } catch (error) {
    return Response.json(
      {
        error:
          error instanceof HermesGatewayError
            ? error.message
            : "hermes gateway is unreachable",
      },
      { status: 502 }
    );
  }
}
