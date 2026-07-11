import type { HostedChatMessage } from "@/lib/hosted-web-device";
import { hasFinalRemoteResponse as sharedHasFinalRemoteResponse } from "@finite/chat-ui";

/**
 * Hermes marks a final or otherwise notify-worthy assistant delivery with
 * metadata `notify=true`; the Rust projection exposes that as
 * `final_delivery`. Complete commentary, tool progress, and ephemeral working
 * activity are deliberately irrelevant.
 */
export function hasFinalRemoteResponse(
  messages: HostedChatMessage[],
  afterSeq: number,
  ownAccountId?: string | null
) {
  return sharedHasFinalRemoteResponse(messages, afterSeq, ownAccountId);
}
