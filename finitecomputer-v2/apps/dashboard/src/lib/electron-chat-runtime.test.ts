import assert from "node:assert/strict";
import test from "node:test";

import {
  ElectronChatStateError,
  electronAttachmentUpload,
  electronChatRuntime,
  reconcileElectronChatState,
  mergeElectronChatState,
  type ElectronLocalDevice,
} from "@/lib/electron-chat-runtime";
import type { HostedAgentBinding, HostedChatState } from "@/lib/hosted-web-device";

const ACCOUNT_ID = "a".repeat(64);
const ROOM_ID = "canonical-room";
const DEVICE: ElectronLocalDevice = {
  status: "ready",
  account_id: ACCOUNT_ID,
  device_id: "electron-device",
};

test("dashboard accepts both released and recovery-capable Electron bridges", (context) => {
  const previousWindow = globalThis.window;
  context.after(() => {
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: previousWindow,
      writable: true,
    });
  });
  const common = {
    ensureLocalDevice: async () => DEVICE,
    daemonState: async () => chatState(ACCOUNT_ID, [ROOM_ID]),
    dispatchDaemonAction: async () => chatState(ACCOUNT_ID, [ROOM_ID]),
    uploadDaemonAttachments: async () => chatState(ACCOUNT_ID, [ROOM_ID]),
    attachmentUrl: () => "finitechat-media://attachment/room/message/attachment",
    onDaemonUpdate: () => () => undefined,
    onDaemonGeneration: () => () => undefined,
    onDaemonError: () => () => undefined,
    onDeviceLinkStatus: () => () => undefined,
  };
  const setBridge = (finiteChatDesktop: unknown) => {
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: { finiteChatDesktop },
      writable: true,
    });
  };

  setBridge({
    ...common,
    version: 1,
    capabilities: ["local-chat-v1", "automatic-device-link-v1"],
  });
  assert.equal(electronChatRuntime()?.version, 1);

  setBridge({
    ...common,
    version: 2,
    capabilities: [
      "local-chat-v1",
      "automatic-device-link-v1",
      "revoked-device-recovery-v1",
    ],
    recoverLocalDevice: async () => DEVICE,
  });
  assert.equal(electronChatRuntime()?.version, 2);

  setBridge({
    ...common,
    version: 2,
    capabilities: ["local-chat-v1", "automatic-device-link-v1"],
  });
  assert.equal(electronChatRuntime(), null);
});

test("local state accepts only the authoritative account and canonical room", () => {
  const hosted = chatState(ACCOUNT_ID, [], binding(ACCOUNT_ID));
  const local = chatState(ACCOUNT_ID, [ROOM_ID]);
  assert.deepEqual(
    mergeElectronChatState(local, hosted, DEVICE).hosted_agent_binding,
    hosted.hosted_agent_binding
  );

  assert.throws(
    () => mergeElectronChatState(chatState("b".repeat(64), [ROOM_ID]), hosted, DEVICE),
    (error) => error instanceof ElectronChatStateError
      && error.code === "account_mismatch"
  );
  assert.throws(
    () => mergeElectronChatState(chatState(ACCOUNT_ID, []), hosted, DEVICE),
    (error) => error instanceof ElectronChatStateError
      && error.code === "canonical_room_missing"
  );
  assert.throws(
    () => mergeElectronChatState(local, chatState(ACCOUNT_ID, []), DEVICE),
    (error) => error instanceof ElectronChatStateError
      && error.code === "missing_binding"
  );
});

test("a missing later-created room is reconciled without a pairing flow", async () => {
  const hosted = chatState(ACCOUNT_ID, [], binding(ACCOUNT_ID));
  const states = [chatState(ACCOUNT_ID, []), chatState(ACCOUNT_ID, [ROOM_ID])];
  const reconciliations: string[] = [];

  const result = await reconcileElectronChatState(
    { daemonState: async () => states.shift()! },
    hosted,
    DEVICE,
    async (targetDeviceId) => {
      reconciliations.push(targetDeviceId);
      return reconciliation();
    },
    { wait: async () => true }
  );

  assert.deepEqual(reconciliations, [DEVICE.device_id]);
  assert.equal(result.hosted_agent_binding?.canonical_room_id, ROOM_ID);
});

test("account mismatches fail closed before room reconciliation", async () => {
  let reconciliations = 0;
  await assert.rejects(
    reconcileElectronChatState(
      { daemonState: async () => chatState("b".repeat(64), []) },
      chatState(ACCOUNT_ID, [], binding(ACCOUNT_ID)),
      DEVICE,
      async () => {
        reconciliations += 1;
        return reconciliation();
      }
    ),
    (error) => error instanceof ElectronChatStateError
      && error.code === "account_mismatch"
  );
  assert.equal(reconciliations, 0);
});

test("room reconciliation validates the account-bound server projection", async () => {
  await assert.rejects(
    reconcileElectronChatState(
      { daemonState: async () => chatState(ACCOUNT_ID, []) },
      chatState(ACCOUNT_ID, [], binding(ACCOUNT_ID)),
      DEVICE,
      async () => ({ ...reconciliation(), target_device_id: "other-device" }),
      { attempts: 1 }
    ),
    /invalid response/u
  );
});

test("attachment forms become narrow structured-clone uploads", async () => {
  const data = new FormData();
  data.set("room_id", ROOM_ID);
  data.set("topic_id", "topic-1");
  data.set("caption", "hello");
  data.append("files", new File([new Uint8Array([1, 2, 3])], "proof.bin", {
    type: "application/octet-stream",
  }));

  const upload = await electronAttachmentUpload(data);
  assert.equal(upload.room_id, ROOM_ID);
  assert.equal(upload.topic_id, "topic-1");
  assert.equal(upload.chat_id, null);
  assert.equal(upload.caption, "hello");
  assert.equal(upload.files[0]?.filename, "proof.bin");
  assert.deepEqual(new Uint8Array(upload.files[0]!.bytes), new Uint8Array([1, 2, 3]));
});

function binding(accountId: string): HostedAgentBinding {
  return {
    version: 1,
    project_id: "project-1",
    human_account_id: accountId,
    agent_account_id: "c".repeat(64),
    agent_npub: "npub-agent",
    canonical_room_id: ROOM_ID,
    associated_room_ids: [],
  };
}

function reconciliation() {
  return {
    project_id: "project-1",
    target_device_id: DEVICE.device_id,
    status: "ready",
    room_count: 1,
    active_room_count: 1,
  };
}

function chatState(
  accountId: string,
  roomIds: string[],
  hostedBinding?: HostedAgentBinding
): HostedChatState {
  return {
    rev: 1,
    identity: { account_id: accountId, device_id: "device-1" },
    rooms: roomIds.map((roomId) => ({
      room_id: roomId,
      display_name: "Agent",
      state: "Connected",
      status: "Connected",
      user_status_text: "",
      last_message_preview: "",
      unread_count: 0,
      can_load_older: false,
      is_agent_chat: true,
    })),
    topics: [],
    status: "Ready",
    messages: [],
    profiles: [],
    devices: [],
    typing_members: [],
    hosted_agent_binding: hostedBinding,
    flow: {
      notice_busy: false,
      scan_in_flight: false,
      scan_result: "",
    },
  };
}
