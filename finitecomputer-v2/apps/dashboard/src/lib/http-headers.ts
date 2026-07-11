export function inlineContentDisposition(filename: string) {
  const fallback = asciiHeaderFilename(filename);
  return `inline; filename="${fallback}"; filename*=UTF-8''${encodeRfc5987Value(filename)}`;
}

/** Destructive browser form posts must originate from this exact dashboard. */
export function requestHasExactOrigin(request: Request, expectedBaseUrl = request.url) {
  const origin = request.headers.get("origin");
  if (!origin) return false;

  try {
    return new URL(origin).origin === new URL(expectedBaseUrl).origin;
  } catch {
    return false;
  }
}

/** Compare Origin to the browser-visible request host, including its port. */
export function requestOriginMatchesHost(request: Request) {
  const origin = request.headers.get("origin");
  const host = request.headers.get("host");
  if (!origin || !host) return false;

  try {
    const originUrl = new URL(origin);
    if (originUrl.host !== host) return false;
    const forwardedProtocol = request.headers
      .get("x-forwarded-proto")
      ?.split(",", 1)[0]
      ?.trim();
    const expectedProtocol = forwardedProtocol
      ? `${forwardedProtocol}:`
      : new URL(request.url).protocol;
    return originUrl.protocol === expectedProtocol;
  } catch {
    return false;
  }
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
