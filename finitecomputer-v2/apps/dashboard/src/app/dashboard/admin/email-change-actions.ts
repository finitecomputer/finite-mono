"use server";

import { randomUUID } from "node:crypto";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";
import { coreEmailChange, loadCoreEmailChangeOperation, lookupCoreEmailChangeTarget } from "@/lib/core-client";
import { advanceEmailChange, type EmailChangePreview, type EmailChangeRequest, type EmailChangeTarget, type EmailChangeUser } from "@/lib/account-email-change";

export type EmailChangeFormState = {
  request?: EmailChangeRequest; preview?: EmailChangePreview; target?: EmailChangeTarget;
  message?: string; error?: string;
};

async function workos<T>(path: string, body?: object): Promise<T> {
  const key = process.env.WORKOS_API_KEY?.trim();
  if (!key) throw new Error("WorkOS is not configured.");
  const response = await fetch(`https://api.workos.com/user_management/${path}`, {
    method: body ? "POST" : "GET", cache: "no-store", signal: AbortSignal.timeout(15_000),
    headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json" },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  // Never surface/log provider response bodies, credentials, or verification codes.
  if (!response.ok) throw new Error(`WorkOS could not complete this step (${response.status}). The code may be expired, the destination occupied, or the account managed by SSO. Resume to check its current state before retrying.`);
  const text = await response.text();
  return text ? JSON.parse(text) as T : undefined as T;
}

export async function accountEmailChangeAction(_previous: EmailChangeFormState, form: FormData): Promise<EmailChangeFormState> {
  let request: EmailChangeRequest | undefined;
  let target: EmailChangeTarget | undefined;
  try {
    if (process.env.ADMIN_ACCOUNT_EMAIL_CHANGE_ENABLED !== "true") throw new Error("Account email changes are not enabled.");
    const viewer = await loadOptionalViewerContext();
    if (!viewer.isAdmin) throw new Error("Only administrators can change account emails.");
    const action = String(form.get("step") ?? "review");
    if (!["review", "send", "confirm", "resume"].includes(action)) throw new Error("Unknown operation.");
    if (action === "review") {
      const oldEmail = String(form.get("oldEmail") ?? "").trim().toLowerCase();
      const newEmail = String(form.get("newEmail") ?? "").trim().toLowerCase();
      target = await lookupCoreEmailChangeTarget(oldEmail);
      if (target.pendingOperationId) {
        request = (await loadCoreEmailChangeOperation(target.pendingOperationId)).request;
        return { request, target, error: "This account has a pending change. Resume it below before starting another." };
      }
      request = { operationId: randomUUID(), userId: target.userId, workosUserId: target.workosUserId,
        expectedEmail: target.email, newEmail, evidenceReference: String(form.get("reference") ?? "").trim() };
    } else if (action === "resume" || action === "confirm") {
      // Reload durable intent; hidden fields and previous action state are untrusted.
      request = (await loadCoreEmailChangeOperation(String(form.get("operationId") ?? ""))).request;
    } else {
      request = JSON.parse(String(form.get("request") ?? "")) as EmailChangeRequest;
    }
    const result = await advanceEmailChange({
      core: coreEmailChange,
      user: (id) => workos<EmailChangeUser>(`users/${encodeURIComponent(id)}`),
      occupied: async (email, subject) => {
        const users = await workos<{ data: EmailChangeUser[] }>(`users?email=${encodeURIComponent(email)}&limit=100`);
        return users.data.some((user) => user.id !== subject);
      },
      send: (id, email) => workos(`users/${encodeURIComponent(id)}/email_change/send`, { new_email: email }),
      confirm: (id, code) => workos(`users/${encodeURIComponent(id)}/email_change/confirm`, { code }),
    }, action as "review" | "send" | "confirm" | "resume", request!, String(form.get("code") ?? "").trim());
    return { request, target, ...result };
  } catch (error) {
    return { request, target, error: error instanceof Error ? error.message : "The operation could not finish. Resume to check its state." };
  }
}
