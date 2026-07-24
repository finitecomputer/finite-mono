import { NextResponse } from "next/server";

import {
  currentAccountDeviceLinkStatus,
  deviceLinkRouteError,
  parseDeviceLinkJsonRequest,
} from "@/lib/device-link";
import { getDeviceLinkAccountAuthContext } from "@/lib/device-link-auth";

export const dynamic = "force-dynamic";

const PRIVATE_NO_STORE_HEADERS = { "cache-control": "private, no-store" };

export async function POST(request: Request) {
  try {
    const input = await parseDeviceLinkJsonRequest(request);
    const account = await getDeviceLinkAccountAuthContext(request);
    return NextResponse.json(await currentAccountDeviceLinkStatus(input, account), {
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
