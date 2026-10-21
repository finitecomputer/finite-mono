import { NextResponse } from "next/server";

import { setCoreFrontDoorPreference } from "@/lib/core-client";
import {
  FRONT_DOOR_FRESH_COOKIE,
  FRONT_DOOR_FRESH_SECONDS,
  FRONT_DOOR_SETTINGS_ANCHOR,
  FRONT_DOOR_STAY_COOKIE,
  FRONT_DOOR_STAY_SECONDS,
  loadFrontDoorConfig,
} from "@/lib/front-door";
import { workosBaseUrl } from "@/lib/workos-auth";

// The front door's stable way back (FRONT_DOOR_SETTINGS_PATH). As a route
// handler it is never wrapped by the redirecting /dashboard layout. With the
// switch off it does not exist, exactly as before.

function notFound() {
  return new NextResponse("Not Found", { status: 404 });
}

function settingsRedirect(request: Request, error = false) {
  const url = new URL("/dashboard", workosBaseUrl() ?? request.url);
  if (error) url.searchParams.set("frontDoorError", "1");
  url.hash = FRONT_DOOR_SETTINGS_ANCHOR;
  return NextResponse.redirect(url, { status: 303 });
}

function dashboardCookie(response: NextResponse, name: string, value: string, maxAge: number) {
  response.cookies.set(name, value, {
    httpOnly: true,
    secure: process.env.NODE_ENV === "production",
    sameSite: "lax",
    path: "/dashboard",
    maxAge,
  });
  return response;
}

/** Open the legacy dashboard for this browser session and show the setting. */
export async function GET(request: Request) {
  if (!loadFrontDoorConfig().enabled) return notFound();
  return dashboardCookie(settingsRedirect(request), FRONT_DOOR_STAY_COOKIE, "1", FRONT_DOOR_STAY_SECONDS);
}

/** Turn "use the new Finite by default" on or off for the signed-in user. */
export async function POST(request: Request) {
  if (!loadFrontDoorConfig().enabled) return notFound();
  const value = (await request.formData()).get("useNewFinite");
  if (value !== "true" && value !== "false") return settingsRedirect(request, true);
  const useNewFinite = value === "true";
  try {
    await setCoreFrontDoorPreference(useNewFinite);
  } catch (error) {
    console.warn("[front-door] could not save the preference", {
      error: error instanceof Error ? error.message : String(error),
    });
    return settingsRedirect(request, true);
  }
  // The choice replaces any "stay for now" visit. Turning it on goes through
  // /dashboard so the layout's stay rules (billing, operators) still apply.
  const response = useNewFinite
    ? NextResponse.redirect(new URL("/dashboard", workosBaseUrl() ?? request.url), { status: 303 })
    : settingsRedirect(request);
  dashboardCookie(response, FRONT_DOOR_STAY_COOKIE, "", 0);
  return dashboardCookie(response, FRONT_DOOR_FRESH_COOKIE, "1", FRONT_DOOR_FRESH_SECONDS);
}
