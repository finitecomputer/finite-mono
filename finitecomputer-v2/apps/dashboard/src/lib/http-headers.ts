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

/** Resolve the browser-visible request origin across a trusted reverse proxy. */
export function browserVisibleRequestOrigin(
  request: { headers: Pick<Headers, "get">; url: string }
) {
  const host = request.headers.get("host")?.trim();
  if (!host || host.includes(",")) return null;

  try {
    const forwardedProtocol = request.headers
      .get("x-forwarded-proto")
      ?.split(",", 1)[0]
      ?.trim();
    const protocol = forwardedProtocol
      ? `${forwardedProtocol}:`
      : new URL(request.url).protocol;
    if (protocol !== "http:" && protocol !== "https:") return null;
    const publicUrl = new URL(`${protocol}//${host}`);
    return publicUrl.host === host ? publicUrl.origin : null;
  } catch {
    return null;
  }
}

/** Compare Origin to the browser-visible request host, including its port. */
export function requestOriginMatchesHost(request: Request) {
  const origin = request.headers.get("origin");
  const publicOrigin = browserVisibleRequestOrigin(request);
  if (!origin || !publicOrigin) return false;

  try {
    return new URL(origin).origin === publicOrigin;
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
