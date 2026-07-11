import assert from "node:assert/strict";
import test from "node:test";

import type { HostedChatState } from "@/lib/hosted-web-device";
import {
  TrustedOwnerClaimLedger,
  type TrustedOwnerClaimScope,
} from "@/lib/trusted-owner-claim";

const scope: TrustedOwnerClaimScope = {
  workosUserId: "user_paul",
  machineId: "agent-sol",
  hostedAccountId: "hosted-account",
  roomId: "room-sol",
  agentAccountId: "agent-account",
};

test("a retained final Agent delivery avoids a redundant blocking owner claim", () => {
  const ledger = new TrustedOwnerClaimLedger();
  const state = stateWithMessage({
    room_id: scope.roomId,
    sender_account_id: scope.agentAccountId,
    final_delivery: true,
  });

  assert.equal(ledger.established(state, scope), true);
  assert.equal(
    ledger.established(state, { ...scope, roomId: "another-room" }),
    false
  );
  assert.equal(
    ledger.established(state, { ...scope, agentAccountId: "another-agent" }),
    false
  );
});

test("new and non-final conversations still require the typed owner claim", () => {
  const ledger = new TrustedOwnerClaimLedger();

  assert.equal(ledger.established(stateWithMessage(), scope), false);
  assert.equal(
    ledger.established(
      stateWithMessage({
        room_id: scope.roomId,
        sender_account_id: scope.agentAccountId,
        final_delivery: false,
      }),
      scope
    ),
    false
  );
});

test("a successful claim is reused for the same exact hosted principal scope", () => {
  const ledger = new TrustedOwnerClaimLedger();
  ledger.remember(scope);

  assert.equal(ledger.established(stateWithMessage(), scope), true);
  assert.equal(
    ledger.established(stateWithMessage(), { ...scope, hostedAccountId: "replacement" }),
    false
  );
});

function stateWithMessage(
  fields?: Partial<HostedChatState["messages"][number]>
): Pick<HostedChatState, "messages"> {
  return {
    messages: fields
      ? [
          {
            room_id: "room",
            seq: 1,
            message_id: "message",
            sender_account_id: "sender",
            sender_device_id: "device",
            sender_display_name: "Agent",
            text: "done",
            display_content: "done",
            kind: "message",
            status: "complete",
            final_delivery: false,
            is_mine: false,
            media: [],
            timestamp_unix_seconds: 1,
            display_timestamp: "now",
            ...fields,
          },
        ]
      : [],
  };
}
