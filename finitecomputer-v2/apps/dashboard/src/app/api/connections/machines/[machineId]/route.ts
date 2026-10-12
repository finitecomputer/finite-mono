import {
  connectionsRouteResponse,
  dispatchAgentConnectionAction,
  loadAgentConnections,
} from "@/lib/hosted-agent-controls";

type RouteContext = { params: Promise<{ machineId: string }> };

export async function GET(_request: Request, context: RouteContext) {
  const { machineId } = await context.params;
  return connectionsRouteResponse(() => loadAgentConnections(machineId));
}

export async function POST(request: Request, context: RouteContext) {
  const { machineId } = await context.params;
  return connectionsRouteResponse(async () => {
    const payload = await request.json().catch(() => null);
    return dispatchAgentConnectionAction(machineId, payload);
  });
}
