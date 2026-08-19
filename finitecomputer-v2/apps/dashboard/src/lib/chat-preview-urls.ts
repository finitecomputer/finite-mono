const MARKDOWN_LINK_PATTERN = /\[[^\]]*\]\(\s*(https?:\/\/[^\s)]+)(?:\s+["'][^"']*["'])?\s*\)/giu;
const BARE_URL_PATTERN = /https?:\/\/[^\s<>()\[\]{}"']+/giu;
const BRAIN_DOC_MARKDOWN_LINK_PATTERN =
  /\[[^\]]*\]\(\s*(brain:\/\/[^\s)]+)(?:\s+["'][^"']*["'])?\s*\)/giu;
const BARE_BRAIN_DOC_PATTERN = /brain:\/\/[^\s<>()\[\]{}"']+/giu;

export function chatPreviewUrls(text: string) {
  const urls: string[] = [];
  const seen = new Set<string>();
  const add = (raw: string) => {
    const value = raw
      .replace(/(?:\*\*|__|~~)+$/u, "")
      .replace(/[.,;:!?]+$/u, "");
    if (!value || seen.has(value)) return;
    seen.add(value);
    urls.push(value);
  };

  for (const match of text.matchAll(MARKDOWN_LINK_PATTERN)) {
    add(match[1]!);
  }
  for (const raw of text.match(BARE_URL_PATTERN) ?? []) {
    add(raw);
  }
  return urls;
}

/**
 * brain:// document URLs in a chat message (markdown links and bare URLs),
 * newest-message-first order left to the caller. Invalid shapes are dropped
 * by the parser, never rendered.
 */
export function chatBrainDocUrls(text: string) {
  const urls: string[] = [];
  const seen = new Set<string>();
  const add = (raw: string) => {
    const value = raw
      .replace(/(?:\*\*|__|~~)+$/u, "")
      .replace(/[.,;:!?]+$/u, "");
    if (!value || seen.has(value)) return;
    seen.add(value);
    urls.push(value);
  };

  for (const match of text.matchAll(BRAIN_DOC_MARKDOWN_LINK_PATTERN)) {
    add(match[1]!);
  }
  for (const raw of text.match(BARE_BRAIN_DOC_PATTERN) ?? []) {
    add(raw);
  }
  return urls;
}
