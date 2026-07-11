import type { HostedChatState } from "@/lib/hosted-web-device";

export type TrustedOwnerClaimScope = {
  workosUserId: string;
  machineId: string;
  hostedAccountId: string;
  roomId: string;
  agentAccountId: string;
};

const MAX_REMEMBERED_CLAIMS = 256;

/**
 * The Agent Runtime remains the authorization authority. This small ledger
 * only prevents the dashboard from synchronously re-running the trusted
 * canary bootstrap command on every page load.
 */
export class TrustedOwnerClaimLedger {
  private readonly remembered = new Map<string, true>();

  established(state: Pick<HostedChatState, "messages">, scope: TrustedOwnerClaimScope) {
    if (this.remembered.has(scopeKey(scope))) return true;

    // A final delivery from this exact Agent in this exact room is durable,
    // encrypted evidence that this retained canary conversation was already
    // usable. The Agent still independently rejects every unauthorized
    // mutation, so this does not grant runtime authority.
    return state.messages.some(
      (message) =>
        message.room_id === scope.roomId &&
        message.sender_account_id === scope.agentAccountId &&
        message.final_delivery
    );
  }

  remember(scope: TrustedOwnerClaimScope) {
    const key = scopeKey(scope);
    this.remembered.delete(key);
    this.remembered.set(key, true);
    while (this.remembered.size > MAX_REMEMBERED_CLAIMS) {
      const oldest = this.remembered.keys().next().value;
      if (typeof oldest !== "string") break;
      this.remembered.delete(oldest);
    }
  }
}

export const trustedOwnerClaims = new TrustedOwnerClaimLedger();

function scopeKey(scope: TrustedOwnerClaimScope) {
  return [
    scope.workosUserId,
    scope.machineId,
    scope.hostedAccountId,
    scope.roomId,
    scope.agentAccountId,
  ].join("\0");
}
