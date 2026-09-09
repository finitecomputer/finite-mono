"use client";

import { useActionState } from "react";
import { accountEmailChangeAction } from "@/app/dashboard/admin/email-change-actions";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export function AdminEmailChangeForm() {
  const [state, action, pending] = useActionState(accountEmailChangeAction, {});
  const request = state.request;
  const completed = state.preview?.status === "completed";
  return (
    <section className="ocean-utility-card mt-4">
      <div className="ocean-utility-card__header">
        <div>
          <h2 className="ocean-utility-card__title">Change sign-in email</h2>
          <p className="text-sm text-muted-foreground">Keep the account and its agents. Google sign-in is supported; the destination email must be unused.</p>
        </div>
      </div>
      <div className="grid gap-4 max-w-2xl">
        {!request && <form action={action} className="grid gap-3">
          <label className="grid gap-1 text-sm">Current sign-in email<Input name="oldEmail" type="email" required autoComplete="off" /></label>
          <label className="grid gap-1 text-sm">New sign-in email<Input name="newEmail" type="email" required autoComplete="off" /></label>
          <label className="grid gap-1 text-sm">Support request reference<Input name="reference" required maxLength={256} placeholder="Issue or support request reference" /></label>
          <Button name="step" value="review" disabled={pending}>Review account</Button>
        </form>}
        {state.error && <p role="alert" className="text-sm text-destructive">{state.error}</p>}
        {state.message && <p role="status" className="text-sm">{state.message}</p>}
        {request && <div className="grid gap-3 rounded-lg border p-4 text-sm">
          <p><strong>{request.expectedEmail}</strong> → <strong>{request.newEmail}</strong></p>
          <dl className="grid gap-1 break-all">
            <div><dt className="inline font-medium">Account: </dt><dd className="inline">{request.userId}</dd></div>
            <div><dt className="inline font-medium">WorkOS user: </dt><dd className="inline">{request.workosUserId}</dd></div>
            <div><dt className="inline font-medium">Operation: </dt><dd className="inline">{request.operationId}</dd></div>
          </dl>
          {state.target && <div><p className="font-medium">Agents</p><ul className="list-disc pl-5">{state.target.projects.map((project) => <li key={project.id}>{project.name}</li>)}</ul>{state.target.projects.length === 0 && <p>No agents on this account.</p>}</div>}
          <p>Existing site access may need re-sharing to the new email. The agent’s connected Gmail/Drive account stays the same.</p>
          {!completed && <>
            <form action={action}>
              <input type="hidden" name="request" value={JSON.stringify(request)} />
              <Button name="step" value="send" disabled={pending}>Send / resend verification code</Button>
            </form>
            <form action={action} className="grid gap-2">
              <input type="hidden" name="operationId" value={request.operationId} />
              <label className="grid gap-1">Code from the new mailbox<Input name="code" inputMode="numeric" autoComplete="one-time-code" pattern="[0-9]{6}" maxLength={6} required /></label>
              <Button name="step" value="confirm" disabled={pending}>Verify and change sign-in email</Button>
            </form>
            <p className="text-muted-foreground">Save the operation ID. If a step fails or times out, resume below to check whether the change already happened.</p>
          </>}
          {completed && <p>Next: sign in with the new Google account, talk to the agent, and ask it to re-share the private site and read its existing Brain share.</p>}
        </div>}
        <form action={action} className="grid gap-2 border-t pt-4">
          <label className="grid gap-1 text-sm">Resume an operation<Input key={request?.operationId ?? "empty"} name="operationId" defaultValue={request?.operationId} required autoComplete="off" /></label>
          <Button name="step" value="resume" variant="outline" disabled={pending}>Check status / resume</Button>
        </form>
        {pending && <p role="status" className="text-sm">Checking the account…</p>}
      </div>
    </section>
  );
}
