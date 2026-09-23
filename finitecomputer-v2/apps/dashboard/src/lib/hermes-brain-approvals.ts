import type { HostedChatMessage } from "./hosted-web-device";

// Same reference-only fbrain trailer consumed by the Finite Chat adapter.
// Hermes owns the durable tool history; Brain owns request details/authority.
const MARKER = /finite-brain-approval-filed brain=([a-z0-9][a-z0-9_-]{0,127}) request=([A-Za-z0-9][A-Za-z0-9_-]{0,127})(?![A-Za-z0-9_-])/g;

export function hermesBrainApprovals(messages: HostedChatMessage[]): HostedChatMessage[] {
  const seen = new Set<string>();
  const pending = new Map<string, { brainId: string; requestId: string }>();
  return messages.map(message => {
    if (message.is_mine) {
      pending.clear(); // An interrupted turn must not lend cards to a later turn.
    } else if (message.kind === "tool") {
      for (const match of (message.display_content || message.text).slice(0, 2 * 1024 * 1024).matchAll(MARKER)) {
        const [, brainId, requestId] = match;
        const key = `${brainId}/${requestId}`;
        if (!seen.has(key) && pending.size < 16) {
          seen.add(key);
          pending.set(key, { brainId, requestId });
        }
      }
    } else if (message.kind === "message" && message.final_delivery && pending.size) {
      const requests = [...pending.values()];
      pending.clear();
      return { ...message, metadata_json: JSON.stringify({ approve: { service: "brain", requests } }) };
    }
    return message;
  });
}
