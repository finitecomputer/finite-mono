import { NextResponse } from "next/server";

import {
  currentHostedWebAccountBinding,
  deviceLinkRouteError,
} from "@/lib/device-link";

export const dynamic = "force-dynamic";

const PRIVATE_NO_STORE_HEADERS = { "cache-control": "private, no-store" };

export async function GET() {
  try {
    return NextResponse.json(await currentHostedWebAccountBinding(), {
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
