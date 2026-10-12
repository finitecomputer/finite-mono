import assert from "node:assert/strict";
import test from "node:test";
import { invitationRecipients, readLaunchInvitation, requireInvitationAccount, sealLaunchInvitation, type LaunchInvitation } from "./launch-invitations";

const env = { WORKOS_COOKIE_PASSWORD: "synthetic-invitation-test-password-at-least-32" };
const invite: LaunchInvitation = { purpose: "organization-seat-v1", email: "person@example.com", organization: "Example enterprise", launchCode: "synthetic-seat-code", hostingTier: "confidential", expiresAtMs: Date.now() + 60_000 };

test("invitation conceals code and binds email, hosting tier, purpose and expiry", async () => {
  const token = await sealLaunchInvitation(invite, env);
  assert(!token.includes(invite.launchCode));
  assert(!token.includes(invite.email));
  assert.deepEqual(await readLaunchInvitation(token, env), invite);
  assert.equal(await readLaunchInvitation(token, env, { nowMs: invite.expiresAtMs }), null);
  assert.deepEqual(await readLaunchInvitation(token, env, { nowMs: invite.expiresAtMs, allowExpiredRedemption: true }), invite);
  assert.equal(await readLaunchInvitation(token.replace(/.$/u, "x"), env), null);
  assert.equal(await readLaunchInvitation(token, { WORKOS_COOKIE_PASSWORD: "different-password-32-characters-or-more" }), null);
  const wrongPurpose = await sealLaunchInvitation({ ...invite, purpose: "agent-draft" as never }, env);
  assert.equal(await readLaunchInvitation(wrongPurpose, env), null);
});

test("only the invited verified WorkOS account can redeem", () => {
  const account = { email: "Person@Example.com", emailVerified: true, workosUserId: "user_synthetic" };
  assert.doesNotThrow(() => requireInvitationAccount(invite, account));
  assert.throws(() => requireInvitationAccount(invite, { ...account, email: "other@example.com" }), /verified email/u);
  assert.throws(() => requireInvitationAccount(invite, { ...account, emailVerified: false }), /verified email/u);
  assert.throws(() => requireInvitationAccount(invite, { ...account, workosUserId: null }), /verified email/u);
});

test("recipient list is optional but otherwise has exactly one distinct email per seat", () => {
  assert.deepEqual(invitationRecipients(null, 2), []);
  assert.deepEqual(invitationRecipients("One@Example.com\ntwo@example.com", 2), ["one@example.com", "two@example.com"]);
  assert.throws(() => invitationRecipients("one@example.com", 2), /each code/u);
  assert.throws(() => invitationRecipients("one@example.com,ONE@example.com", 2), /different/u);
  assert.throws(() => invitationRecipients("not-an-email", 1), /valid/u);
});
