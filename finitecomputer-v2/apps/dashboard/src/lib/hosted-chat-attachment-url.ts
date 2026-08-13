import type { HostedChatMediaAttachment } from "@/lib/hosted-web-device";

export function directHostedImageUrl(
  attachment: Pick<
    HostedChatMediaAttachment,
    "attachment_id" | "kind" | "url"
  >
): string | null {
  if (attachment.kind !== "Image") {
    return null;
  }

  const url = attachment.url?.trim() ?? "";
  // Hermes URL-only media uses the URL itself as its attachment id. Encrypted
  // blob attachments also expose a transport URL, but keep a distinct digest
  // id and must continue through the authenticated download/decryption route.
  if (!url || attachment.attachment_id.trim() !== url) {
    return null;
  }

  try {
    const parsed = new URL(url);
    if (parsed.username || parsed.password) {
      return null;
    }
    if (parsed.protocol === "https:") {
      return url;
    }
    if (parsed.protocol === "http:" && isLoopbackHostname(parsed.hostname)) {
      return url;
    }
  } catch {
    return null;
  }

  return null;
}

function isLoopbackHostname(hostname: string) {
  return (
    hostname === "localhost" ||
    hostname === "127.0.0.1" ||
    hostname === "[::1]"
  );
}
