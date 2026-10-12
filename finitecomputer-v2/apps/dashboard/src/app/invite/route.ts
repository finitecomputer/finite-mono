import { NextRequest, NextResponse } from "next/server";
import { LAUNCH_INVITATION_COOKIE, LAUNCH_INVITATION_TTL, readLaunchInvitation } from "@/lib/launch-invitations";
import { workosBaseUrl } from "@/lib/workos-auth";

// The token is removed from the address before authentication. The HttpOnly
// cookie survives either signup or login; no code is redeemed by visiting a link.
export async function GET(request: NextRequest) {
  const token = request.nextUrl.searchParams.get("token");
  const invitation = await readLaunchInvitation(token);
  if (!invitation || !token) {
    return new NextResponse("This invitation is invalid or expired. Ask your organization for a new link.", {
      status: 400, headers: { "Cache-Control": "no-store", "Referrer-Policy": "no-referrer" },
    });
  }
  const response = NextResponse.redirect(new URL("/dashboard?new=1", workosBaseUrl() ?? request.url), 303);
  response.headers.set("Cache-Control", "no-store");
  response.headers.set("Referrer-Policy", "no-referrer");
  response.cookies.set(LAUNCH_INVITATION_COOKIE, token, {
    httpOnly: true, secure: process.env.NODE_ENV === "production", sameSite: "lax", path: "/dashboard",
    // Core enforces the redemption deadline. Retain an opened invite for exact
    // retries if redemption succeeded just before the batch expired.
    maxAge: LAUNCH_INVITATION_TTL,
  });
  return response;
}

export async function POST(request: NextRequest) {
  const response = NextResponse.redirect(new URL("/dashboard?new=1", workosBaseUrl() ?? request.url), 303);
  response.cookies.set(LAUNCH_INVITATION_COOKIE, "", { path: "/dashboard", maxAge: 0 });
  return response;
}
