import {
  claimOwner,
  hostedAgentContext,
  HostedAgentControlError,
  type AgentCommandContext,
} from "@/lib/hosted-agent-controls";
import {
  hostedDeviceRuntimeCommand,
  HostedDeviceRequestError,
  type HostedRuntimeCommand,
  type HostedRuntimeCommandResponse,
} from "@/lib/hosted-web-device";

export const CHAT_ADMISSION_COMMAND = "chat.admission";
export const CHAT_ADMISSION_SCHEMA = "finite.chat.admission.v1";

// The agent sidecar consumes `chat.admission` in its own inbound scan and
// never posts a runtime-command result (finitechat-cli hermes.rs,
// CHAT_ADMISSION_COMMAND), so the hosted device always ends the wait with its
// result-less timeout — the acceptance script
// scripts/devfinity-chat-authz-upgrade documents that timeout as expected.
// One second is the shortest wait the hosted device clamps to, and the
// sidecar consumes the command inside that window: admission actions must
// never sit on a reply that cannot come.
export const CHAT_ADMISSION_WAIT_MILLIS = 1_000;

const RESULT_LESS_TIMEOUT_PREFIX = "The agent did not respond in time.";
const ADMISSION_ACCOUNT_ID_PATTERN = /^[0-9a-f]{64}$/u;

export type ChatAdmissionAction = {
  action: "grant" | "revoke";
  accountId: string;
};

export type ChatAdmissionChangeResult =
  | { status: "applied" }
  | { status: "sent" };

export function parseChatAdmissionAction(payload: unknown): ChatAdmissionAction {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    throw new HostedAgentControlError("The admission request is invalid.", 400);
  }
  const record = payload as Record<string, unknown>;
  const action = typeof record.action === "string" ? record.action.trim() : "";
  if (action !== "grant" && action !== "revoke") {
    throw new HostedAgentControlError("Choose grant or revoke.", 400);
  }
  const accountId =
    typeof record.accountId === "string" ? record.accountId.trim().toLowerCase() : "";
  // Mirror the sidecar's normalize_admission_account_id so an invalid id
  // fails in the dashboard instead of being silently consumed on the agent.
  if (!ADMISSION_ACCOUNT_ID_PATTERN.test(accountId)) {
    throw new HostedAgentControlError(
      "Enter the account id as 64 hexadecimal characters.",
      400
    );
  }
  return { action, accountId };
}

export function chatAdmissionCommand(
  roomId: string,
  targetAccountId: string,
  change: ChatAdmissionAction
): HostedRuntimeCommand {
  return {
    room_id: roomId,
    target_account_id: targetAccountId,
    command: CHAT_ADMISSION_COMMAND,
    schema: CHAT_ADMISSION_SCHEMA,
    body: { action: change.action, account_id: change.accountId },
    wait_millis: CHAT_ADMISSION_WAIT_MILLIS,
  };
}

export function isResultLessAdmissionTimeout(error: unknown): boolean {
  return (
    error instanceof HostedDeviceRequestError &&
    error.status === 500 &&
    error.message.startsWith(RESULT_LESS_TIMEOUT_PREFIX)
  );
}

/**
 * Send one chat admission change over the owner runtime-command channel.
 * "sent" is the honest terminal state today: the sidecar applies the change
 * without a receipt, so only transport-level failures are observable.
 * "applied" is kept for a terminal succeeded result if the sidecar ever
 * starts posting them.
 */
export async function dispatchChatAdmissionCommand(
  context: AgentCommandContext,
  change: ChatAdmissionAction
): Promise<ChatAdmissionChangeResult> {
  let response: HostedRuntimeCommandResponse;
  try {
    response = await hostedDeviceRuntimeCommand(
      context.config,
      context.account,
      chatAdmissionCommand(context.roomId, context.targetAccountId, change)
    );
  } catch (error) {
    if (isResultLessAdmissionTimeout(error)) {
      return { status: "sent" };
    }
    throw error;
  }
  if (response.status === "succeeded") {
    return { status: "applied" };
  }
  throw new HostedAgentControlError(
    response.error?.message || "The agent could not apply that admission change.",
    response.error?.code === "unauthorized" ? 403 : 502
  );
}

export async function applyChatAdmission(machineId: string, payload: unknown) {
  const change = parseChatAdmissionAction(payload);
  const context = await hostedAgentContext(machineId);
  await claimOwner(context);
  return dispatchChatAdmissionCommand(context, change);
}
