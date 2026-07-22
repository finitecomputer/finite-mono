import { NextResponse } from "next/server";

import {
  currentAccountDeviceLinkStatus,
  deviceLinkRouteError,
  parseDeviceLinkJsonRequest,
} from "@/lib/device-link";

export const dynamic = "force-dynamic";

const PRIVATE_NO_STORE_HEADERS = { "cache-control": "private, no-store" };

export async function POST(request: Request) {
  try {
    const input = await parseDeviceLinkJsonRequest(request);
    return NextResponse.json(await currentAccountDeviceLinkStatus(input), {
      headers: PRIVATE_NO_STORE_HEADERS,
    });
  } catch (error) {
    const safe = deviceLinkRouteError(error);
    return NextResponse.json(
      { error: safe.message },
      { status: safe.status, headers: PRIVATE_NO_STORE_HEADERS }
    );
  }
}
