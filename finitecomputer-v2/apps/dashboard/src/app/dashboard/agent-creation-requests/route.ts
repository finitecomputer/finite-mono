import { revalidatePath } from "next/cache";
import { NextResponse } from "next/server";

import { requestCoreAgentCreation } from "@/lib/core-client";
import { dashboardDevLaunchCode, getAccountAuthContext } from "@/lib/dashboard-auth";
import { workosBaseUrl } from "@/lib/workos-auth";

function dashboardRedirect(request: Request, error?: unknown) {
  const url = new URL("/dashboard", workosBaseUrl() ?? request.url);
  if (error) {
    url.searchParams.set(
      "agentCreationError",
      error instanceof Error ? error.message : "Could not create agent."
    );
  }
  return NextResponse.redirect(url, { status: 303 });
}

export async function POST(request: Request) {
  const formData = await request.formData();
  const account = await getAccountAuthContext();
  console.info("agent creation request received", {
    authenticated: Boolean(account.workosUserId),
    hasEmail: Boolean(account.email),
    emailVerified: account.emailVerified,
    source: account.source,
  });

  try {
    await requestCoreAgentCreation({
      displayName: String(formData.get("displayName") ?? ""),
      // Billing and launch entitlement are server decisions. Never accept a
      // launch code from an untrusted form field.
      launchCode: dashboardDevLaunchCode(account),
      idempotencyKey: String(formData.get("idempotencyKey") ?? ""),
    });
    revalidatePath("/");
    revalidatePath("/dashboard");
    return dashboardRedirect(request);
  } catch (error) {
    return dashboardRedirect(request, error);
  }
}

export async function GET(request: Request) {
  return dashboardRedirect(request);
}
