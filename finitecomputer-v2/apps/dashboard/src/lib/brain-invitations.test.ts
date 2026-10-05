import assert from "node:assert/strict";
import test from "node:test";
import { Children, createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { BrainInvitationCard } from "@/components/brain-action-cards";
import { invitationCardsFromResponse, joinRequestBody } from "@/lib/brain-invitations";

/// Shaped exactly like the Brain server's MyInvitationListResponse
/// (camelCase `MyInvitationResponse`): pending-only rows, no `status` field.
const SERVER_RESPONSE = {
  invitations: [
    {
      id: "invitation-pending",
      inviteCode: "join-code-pending",
      brainId: "acme",
      brainDisplayName: "Acme Research",
      inviterDisplay: "admin@acme.example",
      folderScope: [],
      expiresAt: "2026-10-11T00:00:00Z",
      expired: false,
      publicInstructionsUrl: null,
      originKind: "invitation",
      originRef: null,
    },
    {
      id: "invitation-expired",
      inviteCode: "join-code-expired",
      brainId: "old-brain",
      brainDisplayName: "Old Brain",
      inviterDisplay: "admin@old.example",
      folderScope: ["notes"],
      expiresAt: "2026-09-01T00:00:00Z",
      expired: true,
      publicInstructionsUrl: null,
      originKind: "approval",
      originRef: "approval-event",
    },
  ],
};

function findButton(node: ReactNode): ReactElement<{ onClick?: () => void }> | null {
  if (!isValidElement(node)) return null;
  const element = node as ReactElement<{ children?: ReactNode; onClick?: () => void }>;
  if (element.type === "button") return element;
  for (const child of Children.toArray(element.props.children)) {
    const found = findButton(child);
    if (found) return found;
  }
  return null;
}

test("the server's pending-only rows become cards without a status field", () => {
  const cards = invitationCardsFromResponse(SERVER_RESPONSE);
  assert.deepEqual(
    cards.map((card) => [card.id, card.brainDisplayName, card.expired]),
    [
      ["invitation-pending", "Acme Research", false],
      ["invitation-expired", "Old Brain", true],
    ]
  );
});

test("a pending card shows the sharing text and Join sends the explicit intent", () => {
  const [pending] = invitationCardsFromResponse(SERVER_RESPONSE);
  const html = renderToStaticMarkup(
    createElement(BrainInvitationCard, { card: pending, state: "idle", error: "", onJoin: () => {} })
  );
  assert.match(html, /Acme Research/);
  assert.match(html, /Join Brain/);
  assert.match(html, /admins see your account email/);

  const sent: unknown[] = [];
  const tree = BrainInvitationCard({
    card: pending,
    state: "idle",
    error: "",
    onJoin: (body) => sent.push(body),
  });
  const button = findButton(tree);
  assert.ok(button?.props.onClick, "pending card has a Join button");
  button.props.onClick();
  assert.deepEqual(sent, [{ inviteCode: "join-code-pending", shareAccountContact: true }]);
  assert.deepEqual(joinRequestBody(pending), sent[0]);
});

test("an expired card explains itself and offers no Join", () => {
  const expired = invitationCardsFromResponse(SERVER_RESPONSE)[1];
  const html = renderToStaticMarkup(
    createElement(BrainInvitationCard, { card: expired, state: "idle", error: "", onJoin: () => {} })
  );
  assert.match(html, /This invitation expired/);
  assert.doesNotMatch(html, /Join Brain/);
  assert.doesNotMatch(html, /<button/);
  assert.equal(
    findButton(BrainInvitationCard({ card: expired, state: "idle", error: "", onJoin: () => {} })),
    null
  );
});

test("empty, malformed and unusable responses yield no cards", () => {
  for (const body of [
    null,
    "invitations",
    {},
    { invitations: null },
    { invitations: "x" },
    { invitations: [null, 7, "row"] },
    // Missing or unsafe invite codes, ids or Brain ids are dropped.
    { invitations: [{ id: "a", brainId: "b" }] },
    { invitations: [{ id: "a", brainId: "b", inviteCode: "Bad Code" }] },
    { invitations: [{ id: "a", brainId: "b", inviteCode: "../escape" }] },
    { invitations: [{ id: "", brainId: "b", inviteCode: "ok-code" }] },
    { invitations: [{ id: "a", inviteCode: "ok-code" }] },
  ]) {
    assert.deepEqual(invitationCardsFromResponse(body), [], JSON.stringify(body));
  }
  // A missing display name falls back to the Brain id; non-boolean expiry is
  // not expired.
  const [card] = invitationCardsFromResponse({
    invitations: [{ id: "a", brainId: "b", inviteCode: "ok-code", expired: "true" }],
  });
  assert.equal(card.brainDisplayName, null);
  assert.equal(card.expired, false);
  // The list is bounded.
  const many = {
    invitations: Array.from({ length: 80 }, (_, index) => ({
      id: `invitation-${index}`,
      brainId: "b",
      inviteCode: `code-${index}`,
    })),
  };
  assert.equal(invitationCardsFromResponse(many).length, 50);
});
