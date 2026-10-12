import { connectionsRouteResponse, loadCodexModels } from "@/lib/hosted-agent-controls";

type RouteContext = { params: Promise<{ machineId: string }> };

export async function GET(_request: Request, context: RouteContext) {
  const { machineId } = await context.params;
  return connectionsRouteResponse(() => loadCodexModels(machineId));
}
