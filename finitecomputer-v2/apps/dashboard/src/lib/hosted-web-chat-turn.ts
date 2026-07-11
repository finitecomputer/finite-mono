import type { HostedChatMessage } from "@/lib/hosted-web-device";

/**
 * Hermes marks a final or otherwise notify-worthy assistant delivery with
 * metadata `notify=true`; the Rust projection exposes that as
 * `final_delivery`. Complete commentary, tool progress, and ephemeral working
 * activity are deliberately irrelevant.
 */
export function hasFinalRemoteResponse(
  messages: HostedChatMessage[],
  afterSeq: number
) {
  return messages.some(
    (message) =>
      !message.is_mine
      && message.seq > afterSeq
      && message.final_delivery === true
  );
}
