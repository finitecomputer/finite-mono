export function inlineContentDisposition(filename: string) {
  const fallback = asciiHeaderFilename(filename);
  return `inline; filename="${fallback}"; filename*=UTF-8''${encodeRfc5987Value(filename)}`;
}

function asciiHeaderFilename(filename: string) {
  const fallback = filename
    .normalize("NFKD")
    .replace(/[\r\n"\\;]/g, "_")
    .replace(/[^\x20-\x7E]/g, "_")
    .replace(/\s+/g, " ")
    .trim();

  return fallback || "attachment";
}

function encodeRfc5987Value(value: string) {
  return encodeURIComponent(value).replace(/['()*]/g, (character) =>
    `%${character.charCodeAt(0).toString(16).toUpperCase()}`
  );
}
