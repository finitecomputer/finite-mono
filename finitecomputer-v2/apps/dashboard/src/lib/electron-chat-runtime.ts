import type {
  HostedAgentBinding,
  HostedChatAction,
  HostedChatState,
} from "@/lib/hosted-web-device";

const ELECTRON_CHAT_CAPABILITIES_V1 = [
  "local-chat-v1",
  "automatic-device-link-v1",
] as const;
const ELECTRON_CHAT_CAPABILITIES_V2 = [
  ...ELECTRON_CHAT_CAPABILITIES_V1,
  "revoked-device-recovery-v1",
] as const;

const ELECTRON_ROOM_RECONCILIATION_ATTEMPTS = 8;
const ELECTRON_ROOM_RECONCILIATION_DELAY_MS = 1_000;

export type ElectronChatStateErrorCode =
  | "missing_binding"
  | "account_mismatch"
  | "canonical_room_missing";

export class ElectronChatStateError extends Error {
  constructor(
    readonly code: ElectronChatStateErrorCode,
    message: string
  ) {
    super(message);
    this.name = "ElectronChatStateError";
  }
}

export type ElectronDeviceLinkStatus = {
  status:
    | "preparing"
    | "linking"
    | "joining_rooms"
    | "ready"
    | "recovery_required"
    | "failed";
  message?: string;
};

export type ElectronLocalDevice = {
  status: "ready";
  account_id: string;
  device_id: string;
};

export type ElectronLocalDeviceRecoveryRequired = {
  status: "recovery_required";
  reason: "device_revoked";
  device_id: string;
  message: string;
};

export type ElectronLocalDeviceResult =
  | ElectronLocalDevice
  | ElectronLocalDeviceRecoveryRequired;

export type ElectronAttachmentUpload = {
  room_id: string;
  topic_id?: string | null;
  chat_id?: string | null;
  caption?: string | null;
  reply_to_message_id?: string | null;
  files: Array<{
    filename: string;
    mime_type: string;
    bytes: ArrayBuffer;
  }>;
};

export type ElectronAttachmentAddress = {
  room_id: string;
  message_id: string;
  attachment_id: string;
};

type ElectronChatRuntimeCommon = {
  daemonState(): Promise<HostedChatState>;
  dispatchDaemonAction(action: HostedChatAction): Promise<HostedChatState>;
  uploadDaemonAttachments(upload: ElectronAttachmentUpload): Promise<HostedChatState>;
  attachmentUrl(address: ElectronAttachmentAddress): string;
  onDaemonUpdate(callback: (state: HostedChatState) => void): () => void;
  onDaemonGeneration(callback: (generation: { generation: number }) => void): () => void;
  onDaemonError(callback: (message: string) => void): () => void;
  onDeviceLinkStatus(callback: (status: ElectronDeviceLinkStatus) => void): () => void;
};

export type ElectronChatRuntimeV1 = ElectronChatRuntimeCommon & {
  version: 1;
  capabilities: readonly ["local-chat-v1", "automatic-device-link-v1"];
  ensureLocalDevice(): Promise<ElectronLocalDevice>;
};

export type ElectronChatRuntimeV2 = ElectronChatRuntimeCommon & {
  version: 2;
  capabilities: readonly [
    "local-chat-v1",
    "automatic-device-link-v1",
    "revoked-device-recovery-v1",
  ];
  ensureLocalDevice(): Promise<ElectronLocalDeviceResult>;
  recoverLocalDevice(): Promise<ElectronLocalDeviceResult>;
};

export type ElectronChatRuntime = ElectronChatRuntimeV1 | ElectronChatRuntimeV2;

export type ElectronRoomReconciliation = {
  project_id: string;
  target_device_id: string;
  status: "awaiting_key_package" | "joining_rooms" | "ready";
  room_count: number;
  active_room_count: number;
};

declare global {
  interface Window {
    finiteChatDesktop?: unknown;
  }
}

export function electronChatRuntime(): ElectronChatRuntime | null {
  if (typeof window === "undefined") return null;
  const candidate = window.finiteChatDesktop;
  if (!candidate || typeof candidate !== "object") return null;

  const bridge = candidate as Record<string, unknown>;
  for (const method of [
    "ensureLocalDevice",
    "daemonState",
    "dispatchDaemonAction",
    "uploadDaemonAttachments",
    "attachmentUrl",
    "onDaemonUpdate",
    "onDaemonGeneration",
    "onDaemonError",
    "onDeviceLinkStatus",
  ] as const) {
    if (typeof bridge[method] !== "function") return null;
  }
  if (
    bridge.version === 1
    && hasExactCapabilities(bridge.capabilities, ELECTRON_CHAT_CAPABILITIES_V1)
  ) {
    return candidate as ElectronChatRuntimeV1;
  }
  if (
    bridge.version === 2
    && hasExactCapabilities(bridge.capabilities, ELECTRON_CHAT_CAPABILITIES_V2)
    && typeof bridge.recoverLocalDevice === "function"
  ) {
    return candidate as ElectronChatRuntimeV2;
  }
  return null;
}

export function isElectronLocalDeviceRecoveryRequired(
  value: ElectronLocalDeviceResult
): value is ElectronLocalDeviceRecoveryRequired {
  return value.status === "recovery_required"
    && value.reason === "device_revoked"
    && typeof value.device_id === "string"
    && value.device_id.length > 0;
}

export function mergeElectronChatState(
  local: HostedChatState,
  hosted: HostedChatState,
  device: ElectronLocalDevice
): HostedChatState {
  const binding = hosted.hosted_agent_binding;
  if (!binding) {
    throw new ElectronChatStateError(
      "missing_binding",
      "This agent does not have an authoritative chat binding."
    );
  }
  assertMatchingAccount(local, hosted, binding, device);
  if (!local.rooms.some((room) => room.room_id === binding.canonical_room_id)) {
    throw new ElectronChatStateError(
      "canonical_room_missing",
      "This Device has not joined the agent's canonical chat room."
    );
  }
  return { ...local, hosted_agent_binding: binding };
}

export async function reconcileElectronChatState(
  runtime: Pick<ElectronChatRuntime, "daemonState">,
  hosted: HostedChatState,
  device: ElectronLocalDevice,
  reconcile: (targetDeviceId: string) => Promise<unknown>,
  options: {
    signal?: AbortSignal;
    attempts?: number;
    wait?: (delayMs: number, signal?: AbortSignal) => Promise<boolean>;
  } = {}
): Promise<HostedChatState> {
  let local = await runtime.daemonState();
  try {
    return mergeElectronChatState(local, hosted, device);
  } catch (caught) {
    if (!isElectronChatStateError(caught, "canonical_room_missing")) throw caught;
  }

  const binding = hosted.hosted_agent_binding!;
  const attempts = options.attempts ?? ELECTRON_ROOM_RECONCILIATION_ATTEMPTS;
  if (!Number.isSafeInteger(attempts) || attempts < 1) {
    throw new Error("Electron room reconciliation needs at least one attempt.");
  }
  const wait = options.wait ?? waitForElectronRoomReconciliation;

  for (let attempt = 0; attempt < attempts; attempt += 1) {
    throwIfElectronReconciliationAborted(options.signal);
    parseElectronRoomReconciliation(
      await reconcile(device.device_id),
      binding.project_id,
      device.device_id
    );
    throwIfElectronReconciliationAborted(options.signal);
    local = await runtime.daemonState();
    try {
      return mergeElectronChatState(local, hosted, device);
    } catch (caught) {
      if (!isElectronChatStateError(caught, "canonical_room_missing")) throw caught;
    }
    if (attempt + 1 < attempts && !(await wait(
      ELECTRON_ROOM_RECONCILIATION_DELAY_MS,
      options.signal
    ))) {
      throwIfElectronReconciliationAborted(options.signal);
      break;
    }
  }

  throw new ElectronChatStateError(
    "canonical_room_missing",
    "This Device is still joining the agent's chat room."
  );
}

export function isElectronChatStateError(
  error: unknown,
  code: ElectronChatStateErrorCode
): error is ElectronChatStateError {
  return error instanceof ElectronChatStateError && error.code === code;
}

export async function electronAttachmentUpload(formData: FormData): Promise<ElectronAttachmentUpload> {
  const roomId = requiredFormString(formData, "room_id");
  const entries = formData.getAll("files");
  if (entries.length === 0 || entries.some((file) => typeof file === "string")) {
    throw new Error("Choose at least one attachment to upload.");
  }
  const files = entries.filter((file): file is File => typeof file !== "string");

  return {
    room_id: roomId,
    topic_id: optionalFormString(formData, "topic_id"),
    chat_id: optionalFormString(formData, "chat_id"),
    caption: optionalFormString(formData, "caption"),
    reply_to_message_id: optionalFormString(formData, "reply_to_message_id"),
    files: await Promise.all(files.map(async (file) => ({
      filename: file.name,
      mime_type: file.type || "application/octet-stream",
      bytes: await file.arrayBuffer(),
    }))),
  };
}

function assertMatchingAccount(
  local: HostedChatState,
  hosted: HostedChatState,
  binding: HostedAgentBinding,
  device: ElectronLocalDevice
) {
  const accountId = local.identity.account_id;
  if (
    device.status !== "ready"
    || accountId !== device.account_id
    || accountId !== hosted.identity.account_id
    || accountId !== binding.human_account_id
  ) {
    throw new ElectronChatStateError(
      "account_mismatch",
      "This Device belongs to a different chat account."
    );
  }
}

function parseElectronRoomReconciliation(
  value: unknown,
  expectedProjectId: string,
  expectedDeviceId: string
): ElectronRoomReconciliation {
  if (!value || typeof value !== "object") {
    throw new Error("Device reconciliation returned an invalid response.");
  }
  const record = value as Record<string, unknown>;
  const statuses = new Set<ElectronRoomReconciliation["status"]>([
    "awaiting_key_package",
    "joining_rooms",
    "ready",
  ]);
  if (
    record.project_id !== expectedProjectId
    || record.target_device_id !== expectedDeviceId
    || typeof record.status !== "string"
    || !statuses.has(record.status as ElectronRoomReconciliation["status"])
    || !Number.isSafeInteger(record.room_count)
    || (record.room_count as number) < 0
    || !Number.isSafeInteger(record.active_room_count)
    || (record.active_room_count as number) < 0
    || (record.active_room_count as number) > (record.room_count as number)
  ) {
    throw new Error("Device reconciliation returned an invalid response.");
  }
  return {
    project_id: expectedProjectId,
    target_device_id: expectedDeviceId,
    status: record.status as ElectronRoomReconciliation["status"],
    room_count: record.room_count as number,
    active_room_count: record.active_room_count as number,
  };
}

function waitForElectronRoomReconciliation(delayMs: number, signal?: AbortSignal) {
  if (signal?.aborted) return Promise.resolve(false);
  return new Promise<boolean>((resolve) => {
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", cancel);
      resolve(true);
    }, delayMs);
    const cancel = () => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", cancel);
      resolve(false);
    };
    signal?.addEventListener("abort", cancel, { once: true });
  });
}

function throwIfElectronReconciliationAborted(signal?: AbortSignal) {
  if (!signal?.aborted) return;
  throw signal.reason instanceof Error
    ? signal.reason
    : new Error("Electron room reconciliation was cancelled.");
}

function hasExactCapabilities(
  value: unknown,
  expected: readonly string[]
): value is ElectronChatRuntime["capabilities"] {
  return Array.isArray(value)
    && value.length === expected.length
    && expected.every((capability, index) => value[index] === capability);
}

function requiredFormString(formData: FormData, name: string) {
  const value = formData.get(name);
  if (typeof value !== "string" || !value) {
    throw new Error(`Attachment upload is missing ${name}.`);
  }
  return value;
}

function optionalFormString(formData: FormData, name: string) {
  const value = formData.get(name);
  return typeof value === "string" && value ? value : null;
}
