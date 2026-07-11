import { NextResponse } from "next/server";

import {
  DeviceLinkError,
  currentAccountDeviceLinkStatus,
  parseDeviceLinkJsonRequest,
} from "@/lib/device-link";

export async function POST(request: Request) {
  try {
    const input = await parseDeviceLinkJsonRequest(request);
    return NextResponse.json(await currentAccountDeviceLinkStatus(input), {
      headers: { "cache-control": "private, no-store" },
    });
  } catch (error) {
    const status = error instanceof DeviceLinkError ? error.status : 400;
    const message =
      error instanceof DeviceLinkError
        ? error.message
        : "This device-link request is invalid.";
    return NextResponse.json(
      { error: message },
      { status, headers: { "cache-control": "private, no-store" } }
    );
  }
}
