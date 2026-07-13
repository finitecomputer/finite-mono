import { randomUUID } from "node:crypto";
import { NextResponse } from "next/server";

import { billingCheckoutDestination } from "@/app/actions";
import {
  AGENT_DRAFT_COOKIE,
  AGENT_DRAFT_TTL_SECONDS,
  MAX_AGENT_PROFILE_IMAGE_BYTES,
  agentCreationErrorMessage,
  normalizeAgentDisplayName,
  normalizeAgentReturnMachineId,
  resolveAgentCreationAccessPath,
  sealAgentOnboardingDraft,
  unsealAgentOnboardingDraft,
  type AgentOnboardingDraft,
} from "@/lib/agent-onboarding";
import {
  loadCoreBillingOverview,
  requestCoreAgentCreation,
} from "@/lib/core-client";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import {
  hostedDeviceAuthorizeAgentBinding,
  hostedDeviceConfig,
  hostedDeviceProfileImage,
} from "@/lib/hosted-web-device";
import { stripeBillingStatus } from "@/lib/stripe-billing";
import { workosBaseUrl } from "@/lib/workos-auth";

function dashboardRedirect(
  request: Request,
  error?: unknown,
  creationRequestId?: string,
  returnMachineId?: string | null
) {
  const url = new URL("/dashboard", workosBaseUrl() ?? request.url);
  if (creationRequestId) {
    url.searchParams.set("new", "1");
    url.searchParams.set("creation", creationRequestId);
  }
  if (error) {
    url.searchParams.set("new", "1");
    url.searchParams.set(
      "agentCreationError",
      agentCreationErrorMessage(error)
    );
  }
  if (returnMachineId) {
    url.searchParams.set("machine", returnMachineId);
  }
  return NextResponse.redirect(url, { status: 303 });
}

export async function POST(request: Request) {
  const formData = await request.formData();
  const account = await getAccountAuthContext();
  const returnMachineId = normalizeAgentReturnMachineId(formData.get("machine"));
  let draft: AgentOnboardingDraft | null = null;

  try {
    if (!account.workosUserId || !account.emailVerified) {
      throw new Error("Sign in again to create your agent.");
    }
    const displayName = normalizeAgentDisplayName(formData.get("displayName"));
    const existingDraft = await currentDraft(request, account.workosUserId);
    const profilePictureUrl = await profilePicture(
      formData.get("profilePicture"),
      existingDraft?.profilePictureUrl ?? null,
      account
    );
    // A Core creation can succeed before the Hosted Device accepts its
    // bootstrap authorization. Preserve the signed request identity for an
    // exact retry instead of creating a second project.
    const idempotencyKey =
      existingDraft?.displayName === displayName &&
      existingDraft.profilePictureUrl === profilePictureUrl
        ? existingDraft.idempotencyKey
        : validIdempotencyKey(formData.get("idempotencyKey"));
    draft = {
      version: 1,
      workosUserId: account.workosUserId,
      displayName,
      profilePictureUrl,
      idempotencyKey,
      issuedAtMs: Date.now(),
      returnMachineId,
      stripeCheckoutStartedAtMs: null,
    };
    const billing = await loadCoreBillingOverview({ cacheMode: "fresh" });
    const access = formData.get("access");
    const accessPath = resolveAgentCreationAccessPath(
      access,
      Boolean(billing.billing?.can_create_agent),
      process.env.FC_DASHBOARD_RUNTIME_MODE !== "canary"
    );

    if (accessPath === "launch-code") {
      const launchCode = String(formData.get("launchCode") ?? "").trim();
      if (!launchCode) {
        throw new Error("Enter your Launch Code.");
      }
      const creation = await launchDraft(draft, account, launchCode);
      const response = dashboardRedirect(
        request,
        undefined,
        creation.request.id,
        draft.returnMachineId
      );
      clearDraftCookie(response);
      return response;
    }

    if (accessPath === "stripe") {
      if (!stripeBillingStatus().configured) {
        throw new Error("Payment is unavailable right now.");
      }
      const response = NextResponse.redirect(
        await billingCheckoutDestination(draft.idempotencyKey, draft.returnMachineId),
        { status: 303 }
      );
      setDraftCookie(
        response,
        await sealAgentOnboardingDraft({
          ...draft,
          stripeCheckoutStartedAtMs: Date.now(),
        })
      );
      return response;
    }

    if (accessPath === "entitlement") {
      const creation = await launchDraft(draft, account);
      const response = dashboardRedirect(
        request,
        undefined,
        creation.request.id,
        draft.returnMachineId
      );
      clearDraftCookie(response);
      return response;
    }

    throw new Error("Use a Launch Code or continue to payment.");
  } catch (error) {
    const response = dashboardRedirect(request, error, undefined, returnMachineId);
    if (draft) {
      setDraftCookie(response, await sealAgentOnboardingDraft(draft));
    }
    return response;
  }
}

export async function GET(request: Request) {
  return dashboardRedirect(request);
}

async function launchDraft(
  draft: AgentOnboardingDraft,
  account: Awaited<ReturnType<typeof getAccountAuthContext>>,
  launchCode = ""
) {
  const creation = await requestCoreAgentCreation({
    displayName: draft.displayName,
    launchCode,
    idempotencyKey: draft.idempotencyKey,
    profilePictureUrl: draft.profilePictureUrl,
  });
  if (creation.project.id !== creation.request.project_id) {
    throw new Error("Agent creation returned inconsistent project identity.");
  }
  const device = hostedDeviceConfig();
  if (!device) {
    throw new Error("Chat initialization is unavailable right now.");
  }
  await hostedDeviceAuthorizeAgentBinding(device, account, {
    project_id: creation.request.project_id,
    creation_request_id: creation.request.id,
  });
  return creation;
}

async function profilePicture(
  entry: FormDataEntryValue | null,
  existingUrl: string | null,
  account: Awaited<ReturnType<typeof getAccountAuthContext>>
) {
  if (!(entry instanceof File) || entry.size === 0) return existingUrl;
  if (entry.size > MAX_AGENT_PROFILE_IMAGE_BYTES) {
    throw new Error("Choose an image smaller than 5 MB.");
  }
  if (!entry.type.toLowerCase().startsWith("image/")) {
    throw new Error("Choose an image file.");
  }
  const device = hostedDeviceConfig();
  if (!device) {
    throw new Error("Profile pictures are unavailable right now.");
  }
  return hostedDeviceProfileImage(device, account, entry);
}

function validIdempotencyKey(entry: FormDataEntryValue | null) {
  const value = typeof entry === "string" ? entry.trim() : "";
  return /^[A-Za-z0-9][A-Za-z0-9._:-]{7,127}$/u.test(value) ? value : randomUUID();
}

async function currentDraft(request: Request, workosUserId: string) {
  const cookie = request.headers
    .get("cookie")
    ?.split(";")
    .map((part) => part.trim())
    .find((part) => part.startsWith(`${AGENT_DRAFT_COOKIE}=`))
    ?.slice(AGENT_DRAFT_COOKIE.length + 1);
  return unsealAgentOnboardingDraft(cookie ? decodeURIComponent(cookie) : null, workosUserId);
}

function setDraftCookie(response: NextResponse, sealed: string) {
  response.cookies.set(AGENT_DRAFT_COOKIE, sealed, {
    httpOnly: true,
    secure: process.env.NODE_ENV === "production",
    sameSite: "lax",
    path: "/dashboard",
    maxAge: AGENT_DRAFT_TTL_SECONDS,
  });
}

function clearDraftCookie(response: NextResponse) {
  response.cookies.set(AGENT_DRAFT_COOKIE, "", {
    httpOnly: true,
    secure: process.env.NODE_ENV === "production",
    sameSite: "lax",
    path: "/dashboard",
    maxAge: 0,
  });
}
