import { getSignUpUrl } from "@workos-inc/authkit-nextjs";
import { redirect } from "next/navigation";
import { NextRequest, NextResponse } from "next/server";

import { loadFrontDoorConfig } from "@/lib/front-door";
import { safeWorkosReturnPathname, workosAuthStatus } from "@/lib/workos-auth";

export async function GET(request: NextRequest) {
  // New accounts sign up for the new Finite. Existing accounts still sign in
  // here through /login; the dashboard layout decides where they land.
  const frontDoor = loadFrontDoorConfig();
  if (frontDoor.enabled) {
    redirect(frontDoor.signupUrl);
  }

  const status = workosAuthStatus();

  if (!status.enabled) {
    return NextResponse.json({ error: "Sign up is unavailable" }, { status: 404 });
  }

  if (!status.ready) {
    return NextResponse.json(
      { error: "Sign up is temporarily unavailable" },
      { status: 503 },
    );
  }

  redirect(
    await getSignUpUrl({
      returnTo: safeWorkosReturnPathname(request.nextUrl.searchParams.get("returnTo")),
    }),
  );
}
