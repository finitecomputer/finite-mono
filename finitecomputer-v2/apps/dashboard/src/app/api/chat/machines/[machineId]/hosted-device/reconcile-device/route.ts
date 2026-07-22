import { NextResponse } from "next/server";

import {
  HostedWebChatError,
  hostedWebChatErrorMessage,
  parseHostedDeviceReconcileJsonRequest,
  reconcileHostedWebChatDevice,
} from "@/lib/hosted-web-chat";

export const dynamic = "force-dynamic";

const PRIVATE_NO_STORE_HEADERS = { "cache-control": "private, no-store" };

export async function POST(
  request: Request,
  { params }: { params: Promise<{ machineId: string }> }
) {
  try {
    const { machineId } = await params;
    const { target_device_id: targetDeviceId } =
      await parseHostedDeviceReconcileJsonRequest(request);
    return NextResponse.json(
      await reconcileHostedWebChatDevice(machineId, targetDeviceId),
      { headers: PRIVATE_NO_STORE_HEADERS }
    );
  } catch (error) {
    const status = error instanceof HostedWebChatError ? error.status : 502;
    if (!(error instanceof HostedWebChatError)) {
      console.warn("Hosted web chat Device reconciliation failed", {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return NextResponse.json(
      { error: hostedWebChatErrorMessage(error) },
      { status, headers: PRIVATE_NO_STORE_HEADERS }
    );
  }
}
