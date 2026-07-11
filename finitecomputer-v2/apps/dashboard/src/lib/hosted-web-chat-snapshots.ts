export {
  initialChatSnapshotSource as initialHostedChatSnapshotSource,
  nextChatSnapshotGeneration as nextHostedChatSnapshotGeneration,
  recordChatSnapshot as recordHostedChatSnapshot,
  shouldApplyHttpChatSnapshot as shouldApplyHttpHostedChatSnapshot,
  shouldApplyStreamChatSnapshot as shouldApplyStreamHostedChatSnapshot,
  type ChatSnapshotSource as HostedChatSnapshotSource,
} from "@finite/chat-ui";
