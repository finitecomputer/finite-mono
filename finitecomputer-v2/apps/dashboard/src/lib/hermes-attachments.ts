import type { HostedChatMediaAttachment } from "./hosted-web-device";

/** Hermes persists upload references in message text. Only whole reference
 * lines become cards; ordinary prose/code mentioning a path remains text. */
export function hermesMessageAttachments(text: string): { text: string; media: HostedChatMediaAttachment[] } {
  const media: HostedChatMediaAttachment[] = [];
  let fence = "";
  const body = text.split("\n").map(line => {
    const marker = line.match(/^\s*(`{3,}|~{3,})/);
    if (marker) {
      if (!fence) fence = marker[1];
      else if (marker[1][0] === fence[0] && marker[1].length >= fence.length) fence = "";
      return line;
    }
    if (fence) return line;
    return line.replace(/^@(image|file):(?:`([^`\n]+)`|"([^"\n]+)"|'([^'\n]+)'|([^\s]+))\s*$/gm,
    (line, kind: string, backtick: string, double: string, single: string, bare: string) => {
      const path = backtick || double || single || bare;
      if (!path.startsWith("/") || /[\x00-\x1f]/.test(path)) return line;
      const filename = path.split("/").at(-1) || "Attachment";
      const extension = filename.split(".").at(-1)?.toLowerCase();
      const mime = ({ png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", avif: "image/avif", mp3: "audio/mpeg", wav: "audio/wav", m4a: "audio/mp4", ogg: "audio/ogg", webm: "video/webm", mp4: "video/mp4" } as Record<string, string>)[extension ?? ""];
      if (!media.some(item => item.attachment_id === path)) media.push({ attachment_id: path, filename, kind: kind === "image" ? "Image" : "File", mime_type: mime ?? "application/octet-stream" });
      return "";
    });
  }).join("\n");
  return { text: body.trim(), media };
}

export function hermesAttachmentUrl(runtimeId: string, path: string) {
  return `/api/agents/${encodeURIComponent(runtimeId)}/hermes-file?${new URLSearchParams({ path })}`;
}
