import {
  directChatCanonicalRoomId,
  encodeMultipartBody,
  npubFromAccountId,
  type DirectBridgeRequest,
  type DirectChatAccess,
  type DirectChatBridge,
  type DirectChatTarget,
} from "@/lib/direct-chat-bridge";
import { hostedRequesterAssertion } from "@/lib/hosted-requester-assertion";
import { HostedWebChatError } from "@/lib/hosted-web-chat-error";
import {
  HostedDeviceRequestError,
  hostedDeviceOpenAgentBinding,
  hostedDeviceState,
  type HostedAgentBinding,
  type HostedChatAction,
  type HostedChatState,
  type HostedRuntimeCommand,
  type HostedRuntimeCommandResponse,
} from "@/lib/hosted-web-device";

/*
 * Web chat operations against an agent's own Finite Chat bridge. The agent
 * returns the same AppState JSON as the Hosted Web Device, so browser-facing
 * shapes do not change. Agent-binding ensure/bootstrap/recovery do not exist
 * here: the canonical room is the agent's own report, or, when the agent
 * holds several rooms, the hosted device's existing binding for the project.
 */

const DIRECT_CHAT_TIMEOUT_MS = 15_000;
const DIRECT_CHAT_RUNTIME_COMMAND_TIMEOUT_MS = 65_000;
const DIRECT_CHAT_UPLOAD_TIMEOUT_MS = 120_000;
/** The agent plugin's request cap; the bridge enforces per-file limits. */
export const DIRECT_CHAT_MAX_UPLOAD_BYTES = 70 * 1024 * 1024;
const NEW_CHAT_OUTSIDE_AGENT_CONVERSATION = "New chats must stay in the Agent conversation.";

export type DirectChatSession = {
  bridge: DirectChatBridge;
  access: DirectChatAccess;
  target: DirectChatTarget;
};

type NewChatTarget = Extract<HostedChatAction, { StartTopicChatIntent: unknown }>["StartTopicChatIntent"];

/** The browser's first read: AppState plus the binding the UI scopes chat to. */
export async function directChatBootstrapState(session: DirectChatSession): Promise<HostedChatState> {
  const roomId = directChatRoomId(session);
  const state = await withChatIdentity(session, () => directJson<HostedChatState>(session, {
    method: "GET",
    path: "/v1/app/state",
  }));
  return { ...withoutBinding(state), hosted_agent_binding: directChatBinding(session, state, roomId) };
}

/** A tab-scoped transcript read; never replaces the bootstrap binding. */
export async function directChatViewState(session: DirectChatSession, viewQuery: string) {
  return withoutBinding(await directJson<HostedChatState>(session, {
    method: "GET",
    path: `/v1/app/state${viewQuery}`,
  }));
}

export async function directChatAction(
  session: DirectChatSession,
  action: HostedChatAction,
  options: { textSend: boolean }
) {
  if ("StartTopicChatIntent" in action) {
    return directChatNewChat(session, action.StartTopicChatIntent);
  }
  return withoutBinding(await directJson<HostedChatState>(session, {
    method: "POST",
    path: "/v1/app/actions",
    body: JSON.stringify(action),
    contentType: "application/json",
    requester: options.textSend
      ? (signerAccountId) => createDirectRequesterContext(session, signerAccountId)
      : undefined,
  }));
}

async function directChatNewChat(session: DirectChatSession, target: NewChatTarget) {
  const roomId = directChatRoomId(session);
  if (target.room_id !== roomId) {
    throw new HostedWebChatError(NEW_CHAT_OUTSIDE_AGENT_CONVERSATION, 409);
  }
  const state = await directJson<HostedChatState>(session, {
    method: "GET",
    path: `/v1/app/state?${new URLSearchParams({ room_id: roomId })}`,
  });
  if (!state.topics.some((topic) => topic.room_id === roomId && topic.topic_id === target.topic_id)) {
    throw new HostedWebChatError(NEW_CHAT_OUTSIDE_AGENT_CONVERSATION, 409);
  }
  return withoutBinding(await directJson<HostedChatState>(session, {
    method: "POST",
    path: "/v1/app/new-chat",
    body: JSON.stringify({
      room_id: target.room_id,
      topic_id: target.topic_id,
      reason: target.reason ?? null,
      intent_key: target.intent_key,
    }),
    contentType: "application/json",
  }));
}

/** The raw SSE response; the route pipes it to the browser unchanged. */
export function directChatUpdates(session: DirectChatSession, signal: AbortSignal, viewQuery = "") {
  return session.bridge.request(session.access, session.target, {
    method: "GET",
    path: `/v1/app/updates${viewQuery}`,
    accept: "text/event-stream",
    signal,
  });
}

/** Multipart bytes are built once, hashed, signed, and sent exactly. */
export async function directChatUploadAttachments(session: DirectChatSession, formData: FormData) {
  const { contentType, body } = await encodeMultipartBody(formData);
  if (body.byteLength > DIRECT_CHAT_MAX_UPLOAD_BYTES) {
    throw new HostedWebChatError("Attachments are too large to upload.", 413);
  }
  return withoutBinding(await directJson<HostedChatState>(session, {
    method: "POST",
    path: "/v1/app/attachments",
    body,
    contentType,
    timeoutMs: DIRECT_CHAT_UPLOAD_TIMEOUT_MS,
  }));
}

export function directChatAttachment(
  session: DirectChatSession,
  roomId: string,
  messageId: string,
  attachmentId: string,
  signal: AbortSignal
) {
  const path = [roomId, messageId, attachmentId]
    .map((value) => encodeURIComponent(value))
    .join("/");
  return session.bridge.request(session.access, session.target, {
    method: "GET",
    path: `/v1/app/attachments/${path}`,
    accept: "*/*",
    signal,
  });
}

export function directChatRuntimeCommand(session: DirectChatSession, command: HostedRuntimeCommand) {
  return directJson<HostedRuntimeCommandResponse>(session, {
    method: "POST",
    path: "/v1/app/runtime-commands",
    body: JSON.stringify(command),
    contentType: "application/json",
    timeoutMs: DIRECT_CHAT_RUNTIME_COMMAND_TIMEOUT_MS,
  });
}

/** The canonical room; an empty agent is re-probed on the next request. */
/**
 * A direct session whose canonical room is settled. An agent with exactly
 * one room names it; with several, the hosted device's existing binding for
 * this project names the conversation the person has been using, and it must
 * be one of the agent's rooms. Anything else stays unresolved and fails
 * closed when used.
 */
export async function directChatSessionFor(
  bridge: DirectChatBridge,
  access: DirectChatAccess,
  target: DirectChatTarget
): Promise<DirectChatSession> {
  const { info } = target;
  if (info.canonicalRoomId || info.rooms.length < 2) return { bridge, access, target };
  const bound = await hostedDeviceOpenAgentBinding(access.hosted, access.account, access.projectId)
    .then((state) => state.hosted_agent_binding?.canonical_room_id ?? null)
    .catch(() => null);
  if (!bound || !info.rooms.includes(bound)) return { bridge, access, target };
  return { bridge, access, target: { ...target, info: { ...info, canonicalRoomId: bound } } };
}

export function directChatRoomId(session: DirectChatSession) {
  try {
    return directChatCanonicalRoomId(session.target.info);
  } catch (error) {
    if (session.target.info.rooms.length === 0) session.bridge.forget(session.access.runtimeId);
    throw error;
  }
}

export function createDirectRequesterContext(session: DirectChatSession, signerAccountId: string) {
  return hostedRequesterAssertion(session.access.account, async () => ({
    requesterNpub: signerAccountId,
    agentNpub: npubFromAccountId(session.target.info.agentAccountId),
  }));
}

function directChatBinding(
  session: DirectChatSession,
  state: HostedChatState,
  roomId: string
): HostedAgentBinding {
  const { info } = session.target;
  return {
    version: 1,
    project_id: session.access.projectId,
    human_account_id: state.identity?.account_id ?? "",
    agent_account_id: info.agentAccountId,
    agent_npub: npubFromAccountId(info.agentAccountId),
    canonical_room_id: roomId,
    associated_room_ids: info.rooms.filter((room) => room !== roomId),
  };
}

/**
 * Parity with the legacy bootstrap, which always read hosted-device state:
 * that read mints the user's chat identity on first contact. The signing
 * route itself never mints one.
 */
async function withChatIdentity<T>(session: DirectChatSession, operation: () => Promise<T>) {
  try {
    return await operation();
  } catch (error) {
    if (
      !(error instanceof HostedDeviceRequestError) ||
      error.status !== 409 ||
      error.kind !== "chat_identity_missing"
    ) {
      throw error;
    }
    await hostedDeviceState(session.access.hosted, session.access.account);
    return operation();
  }
}

function directJson<T>(session: DirectChatSession, input: DirectBridgeRequest) {
  return session.bridge.json<T>(session.access, session.target, {
    timeoutMs: DIRECT_CHAT_TIMEOUT_MS,
    ...input,
  });
}

/** Only the bootstrap read carries the binding the dashboard derives. */
function withoutBinding(state: HostedChatState): HostedChatState {
  const next = { ...state };
  delete next.hosted_agent_binding;
  return next;
}
