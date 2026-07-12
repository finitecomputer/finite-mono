import { NextResponse } from "next/server";

import {
  AGENT_DRAFT_COOKIE,
  draftStartedStripeCheckout,
  unsealAgentOnboardingDraft,
} from "@/lib/agent-onboarding";
import {
  loadCoreBillingOverview,
  requestCoreAgentCreation,
} from "@/lib/core-client";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { workosBaseUrl } from "@/lib/workos-auth";

export async function GET(request: Request) {
  const account = await getAccountAuthContext();
  const sealed = request.headers
    .get("cookie")
    ?.split(";")
    .map((part) => part.trim())
    .find((part) => part.startsWith(`${AGENT_DRAFT_COOKIE}=`))
    ?.slice(AGENT_DRAFT_COOKIE.length + 1);
  const draft = await unsealAgentOnboardingDraft(
    sealed ? decodeURIComponent(sealed) : null,
    account.workosUserId
  );
  const dashboard = new URL("/dashboard", workosBaseUrl() ?? request.url);
  if (!draftStartedStripeCheckout(draft)) {
    return NextResponse.redirect(dashboard, { status: 303 });
  }

  try {
    const billing = await loadCoreBillingOverview({ cacheMode: "fresh" });
    if (
      !billing.billing?.can_create_agent ||
      billing.billing.customer_org.billing_class !== "standard" ||
      billing.billing.requires_billing
    ) {
      dashboard.searchParams.set("new", "1");
      dashboard.searchParams.set("billing", "success");
      if (draft.returnMachineId) {
        dashboard.searchParams.set("machine", draft.returnMachineId);
      }
      return NextResponse.redirect(dashboard, { status: 303 });
    }
    const creation = await requestCoreAgentCreation({
      displayName: draft.displayName,
      launchCode: "",
      idempotencyKey: draft.idempotencyKey,
      profilePictureUrl: draft.profilePictureUrl,
    });
    dashboard.searchParams.set("new", "1");
    dashboard.searchParams.set("creation", creation.request.id);
    if (draft.returnMachineId) {
      dashboard.searchParams.set("machine", draft.returnMachineId);
    }
    const response = NextResponse.redirect(dashboard, { status: 303 });
    response.cookies.set(AGENT_DRAFT_COOKIE, "", {
      httpOnly: true,
      secure: process.env.NODE_ENV === "production",
      sameSite: "lax",
      path: "/dashboard",
      maxAge: 0,
    });
    return response;
  } catch (error) {
    dashboard.searchParams.set("new", "1");
    if (draft.returnMachineId) {
      dashboard.searchParams.set("machine", draft.returnMachineId);
    }
    dashboard.searchParams.set(
      "agentCreationError",
      error instanceof Error ? error.message : "Could not create agent."
    );
    return NextResponse.redirect(dashboard, { status: 303 });
  }
}
