const MARKDOWN_LINK_PATTERN = /\[[^\]]*\]\(\s*(https?:\/\/[^\s)]+)(?:\s+["'][^"']*["'])?\s*\)/giu;
const BARE_URL_PATTERN = /https?:\/\/[^\s<>()\[\]{}"']+/giu;

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
