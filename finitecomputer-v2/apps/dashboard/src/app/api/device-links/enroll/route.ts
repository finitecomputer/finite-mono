import { NextResponse } from "next/server";

import {
  deviceLinkRouteError,
  parseDeviceEnrollmentJsonRequest,
  resumeDeviceEnrollment,
} from "@/lib/device-link";

export const dynamic = "force-dynamic";

const PRIVATE_NO_STORE_HEADERS = { "cache-control": "private, no-store" };

export async function POST(request: Request) {
  try {
    const input = await parseDeviceEnrollmentJsonRequest(request);
    return NextResponse.json(await resumeDeviceEnrollment(input), {
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
