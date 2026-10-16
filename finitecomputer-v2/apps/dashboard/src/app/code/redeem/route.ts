import { NextResponse } from "next/server";

import { agentCreationErrorMessage, normalizeAgentDisplayName } from "@/lib/agent-onboarding";
import { requestCoreAgentCreation } from "@/lib/core-client";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { hostedDeviceAuthorizeAgentBinding, hostedDeviceConfig, hostedDeviceOwnerChatAccountId } from "@/lib/hosted-web-device";
import { workosBaseUrl } from "@/lib/workos-auth";

export async function POST(request: Request) {
  const origin = request.headers.get("origin");
  if (origin !== (workosBaseUrl() ?? new URL(request.url).origin) ||
      !request.headers.get("content-type")?.startsWith("application/json")) {
    return NextResponse.json({ error: "Invalid request." }, { status: 403 });
  }
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    return NextResponse.json({ error: "Sign in with a verified email to continue." }, { status: 401 });
  }
  try {
    const input = await request.json();
    const displayName = normalizeAgentDisplayName(input.displayName);
    const launchCode = typeof input.launchCode === "string" ? input.launchCode.trim() : "";
    const idempotencyKey = typeof input.idempotencyKey === "string" ? input.idempotencyKey : "";
    if (!launchCode || launchCode.length > 256) throw new Error("Enter your organization launch code.");
    if (!/^[A-Za-z0-9][A-Za-z0-9._:-]{7,127}$/u.test(idempotencyKey)) throw new Error("Reload this page and try again.");
    const device = hostedDeviceConfig();
    if (!device) throw new Error("Chat initialization is unavailable right now. Please try again.");
    const ownerChatAccountId = await hostedDeviceOwnerChatAccountId(device, account);
    const creation = await requestCoreAgentCreation({
      displayName, launchCode, idempotencyKey, ownerChatAccountId,
      // Core derives Standard or Confidential from the issued code.
    });
    if (creation.project.id !== creation.request.project_id) throw new Error("Agent creation returned inconsistent project identity.");
    await hostedDeviceAuthorizeAgentBinding(device, account, {
      project_id: creation.request.project_id,
      creation_request_id: creation.request.id,
    });
    return NextResponse.json({ location: `/dashboard?new=1&creation=${encodeURIComponent(creation.request.id)}` });
  } catch (error) {
    return NextResponse.json({ error: agentCreationErrorMessage(error) }, { status: 400 });
  }
}
