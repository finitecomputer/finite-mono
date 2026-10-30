import { proxyAgentExportDownload } from "@/lib/agent-export-download";

export const dynamic = "force-dynamic";

type RouteContext = { params: Promise<{ runtimeId: string }> };

export async function GET(request: Request, context: RouteContext) {
  const { runtimeId } = await context.params;
  return proxyAgentExportDownload(request, runtimeId);
}
