/** Durable Core intent coordinates the operator flow. WorkOS and Core are not atomic. */
export type EmailChangeRequest = {
  operationId: string; userId: string; workosUserId: string;
  expectedEmail: string; newEmail: string; evidenceReference: string;
};
export type EmailChangePreview = {
  request: EmailChangeRequest; status: string; projectIds: string[];
  customerOrgIds: string[]; finitePrivateGrantIds: string[];
  blockers: string[]; externalChecksRequired: string[];
};
export type EmailChangeTarget = {
  userId: string; workosUserId: string; email: string;
  projects: { id: string; name: string }[]; pendingOperationId: string | null;
};
export type EmailChangeUser = { id: string; email: string; email_verified: boolean };
export type EmailChangeDependencies = {
  core: (action: "preview" | "prepare" | "complete", request: EmailChangeRequest) => Promise<EmailChangePreview>;
  user: (id: string) => Promise<EmailChangeUser>;
  occupied: (email: string, subject: string) => Promise<boolean>;
  send: (id: string, email: string) => Promise<void>;
  confirm: (id: string, code: string) => Promise<void>;
};
export type EmailChangeStep = { preview: EmailChangePreview; message: string };

function checkedUser(user: EmailChangeUser, request: EmailChangeRequest) {
  if (user.id !== request.workosUserId || !user.email_verified) {
    throw new Error("The original account must still have a verified WorkOS identity.");
  }
  return user.email.trim().toLowerCase();
}

export async function advanceEmailChange(
  deps: EmailChangeDependencies, action: "review" | "send" | "confirm" | "resume",
  request: EmailChangeRequest, code = "",
): Promise<EmailChangeStep> {
  // Core authorization precedes even provider reads. Never trust a dashboard UI gate.
  let preview = await deps.core("preview", request);
  if (preview.status === "cancelled") throw new Error("This operation was cancelled.");
  if (preview.status === "completed") {
    return { preview, message: "Email change complete. Ask the user to sign out and sign in with the new Google account." };
  }
  const email = checkedUser(await deps.user(request.workosUserId), request);
  if (email === request.newEmail && preview.status === "prepared") {
    if (action === "review") return { preview, message: "WorkOS has changed. Resume this operation to finish the Core update." };
    preview = await deps.core("complete", request);
    return { preview, message: "Email change complete. Ask the user to sign out and sign in with the new Google account." };
  }
  if (email !== request.expectedEmail) throw new Error("WorkOS no longer matches this operation. Stop and review the account.");
  if (preview.blockers.length || await deps.occupied(request.newEmail, request.workosUserId)) {
    throw new Error("The destination is occupied or another change is pending. Account merging and deletion are not supported here.");
  }
  if (action === "review" || action === "resume") {
    return { preview, message: preview.status === "prepared" ? "Awaiting verification. Enter the code or resend it." : "Review the account below before sending a verification code." };
  }
  if (action === "send") {
    preview = await deps.core("prepare", request);
    await deps.send(request.workosUserId, request.newEmail);
    return { preview, message: "Verification code sent to the new email address. Ask the user for the code." };
  }
  if (preview.status !== "prepared") throw new Error("Send a verification code before confirming this change.");
  if (!/^\d{6}$/.test(code)) throw new Error("Enter the six-digit verification code.");
  await deps.confirm(request.workosUserId, code);
  // Do not trust a successful mutation response as proof of the resulting identity.
  if (checkedUser(await deps.user(request.workosUserId), request) !== request.newEmail) {
    throw new Error("WorkOS did not return the expected verified email. Resume to check the operation.");
  }
  preview = await deps.core("complete", request);
  return { preview, message: "Email change complete. Ask the user to sign out and sign in with the new Google account." };
}
