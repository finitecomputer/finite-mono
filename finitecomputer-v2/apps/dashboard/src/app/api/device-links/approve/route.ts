import { NextResponse } from "next/server";

import {
  DeviceLinkError,
  approveCurrentAccountDeviceLink,
  parseDeviceLinkJsonRequest,
} from "@/lib/device-link";

export async function POST(request: Request) {
  try {
    const input = await parseDeviceLinkJsonRequest(request);
    return noStoreJson(await approveCurrentAccountDeviceLink(input));
  } catch (error) {
    return deviceLinkError(error);
  }
}

function noStoreJson(value: unknown) {
  return NextResponse.json(value, {
    headers: { "cache-control": "private, no-store" },
  });
}

function deviceLinkError(error: unknown) {
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
