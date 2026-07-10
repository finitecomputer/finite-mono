import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

const MAX_PREVIEW_URL_BYTES = 2 * 1024;
const MAX_RETURN_TO_BYTES = 1024;
const SITES_REQUEST_TIMEOUT_MS = 5_000;
export const MAX_SITE_PREVIEW_REQUEST_BYTES = 4 * 1024;
const SITE_PREVIEW_BODY_TIMEOUT_MS = 5_000;

export type SitePreviewTarget = {
  outputUrl: string;
  returnTo: string;
  originalUrl: string;
};

export class SitePreviewError extends Error {
  constructor(
    message: string,
    readonly status: number
  ) {
    super(message);
  }
}

export async function readBoundedSitePreviewUrl(
  request: Request,
  maxBytes = MAX_SITE_PREVIEW_REQUEST_BYTES,
  timeoutMs = SITE_PREVIEW_BODY_TIMEOUT_MS,
) {
  const declaredLength = request.headers.get("content-length");
  if (declaredLength && (!/^\d+$/u.test(declaredLength) || Number(declaredLength) > maxBytes)) {
    throw new SitePreviewError("Choose a Finite site to preview.", 413);
  }
  const reader = request.body?.getReader();
  if (!reader) throw new SitePreviewError("Choose a Finite site to preview.", 400);
  const controller = new AbortController();
  const abortForClient = () => controller.abort();
  request.signal.addEventListener("abort", abortForClient, { once: true });
  if (request.signal.aborted) controller.abort();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  const chunks: Uint8Array[] = [];
  let length = 0;
  try {
    while (true) {
      const { done, value } = await readSitePreviewChunk(reader, controller.signal);
      if (done) break;
      length += value.byteLength;
      if (length > maxBytes) {
        await reader.cancel().catch(() => undefined);
        throw new SitePreviewError("Choose a Finite site to preview.", 413);
      }
      chunks.push(value);
    }
  } catch (error) {
    if (controller.signal.aborted) {
      throw new SitePreviewError("The site preview request took too long.", 408);
    }
    throw error;
  } finally {
    clearTimeout(timeout);
    request.signal.removeEventListener("abort", abortForClient);
  }
  const body = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    body.set(chunk, offset);
    offset += chunk.byteLength;
  }
  let payload: unknown;
  try {
    payload = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(body)) as unknown;
  } catch {
    throw new SitePreviewError("Choose a Finite site to preview.", 400);
  }
  return payload && typeof payload === "object"
    ? (payload as { url?: unknown }).url
    : undefined;
}

async function readSitePreviewChunk(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  signal: AbortSignal,
) {
  if (signal.aborted) {
    void reader.cancel();
    throw new DOMException("Request body read aborted", "AbortError");
  }
  let abort: (() => void) | undefined;
  const aborted = new Promise<never>((_, reject) => {
    abort = () => {
      reject(new DOMException("Request body read aborted", "AbortError"));
      void reader.cancel();
    };
    signal.addEventListener("abort", abort, { once: true });
  });
  try {
    return await Promise.race([reader.read(), aborted]);
  } finally {
    if (abort) signal.removeEventListener("abort", abort);
  }
}

export async function createSitePreviewSession(machineId: string, rawUrl: unknown) {
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.email || !account.emailVerified) {
    throw new SitePreviewError("Sign in with a verified email to preview this site.", 401);
  }
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access || access.machineId !== machineId) {
    throw new SitePreviewError("Agent not found.", 404);
  }

  const target = parseSitePreviewTarget(rawUrl);
  const upstream = sitesUpstreamOrigin();
  const serviceToken = process.env.FINITE_SITES_VIEWER_SESSION_TOKEN?.trim();
  if (!upstream || !serviceToken) {
    throw new SitePreviewError("Site previews aren't available right now.", 503);
  }

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), SITES_REQUEST_TIMEOUT_MS);
  try {
    const response = await fetch(`${upstream}/internal/v1/viewer-sessions`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${serviceToken}`,
        "content-type": "application/json",
      },
      body: JSON.stringify({
        output_url: target.outputUrl,
        verified_email: account.email,
        return_to: target.returnTo,
      }),
      cache: "no-store",
      signal: controller.signal,
    });
    if (!response.ok) {
      if (response.status === 403) {
        // A validated direct URL still gives public outputs and Sites' own
        // sign-in page a useful preview. It creates no account-backed access.
        return { url: target.originalUrl, originalUrl: target.originalUrl };
      }
      const status = response.status === 401 ? response.status : 502;
      throw new SitePreviewError(
        "Site previews aren't available right now.",
        status
      );
    }
    const payload = (await response.json()) as unknown;
    return {
      url: parseViewerSessionResponse(payload, target),
      originalUrl: target.originalUrl,
    };
  } catch (error) {
    if (error instanceof SitePreviewError) throw error;
    throw new SitePreviewError("Site previews aren't available right now.", 502);
  } finally {
    clearTimeout(timeout);
  }
}

export function sitesUpstreamOrigin(value = process.env.FC_SITES_UPSTREAM_URL) {
  const candidate = value?.trim().replace(/\/$/u, "");
  if (!candidate) return null;
  try {
    const url = new URL(candidate);
    if (
      (url.protocol !== "http:" && url.protocol !== "https:") ||
      url.pathname !== "/" ||
      url.search ||
      url.hash ||
      url.username ||
      url.password
    ) {
      return null;
    }
    return url.origin;
  } catch {
    return null;
  }
}

export function parseSitePreviewTarget(
  value: unknown,
  options: { allowLocalOutputs?: boolean } = {}
): SitePreviewTarget {
  if (typeof value !== "string" || !value || value.length > MAX_PREVIEW_URL_BYTES) {
    throw new SitePreviewError("Choose a Finite site to preview.", 400);
  }
  if (value.includes("\\") || /[\u0000-\u001f\u007f]/u.test(value)) {
    throw new SitePreviewError("Choose a Finite site to preview.", 400);
  }

  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new SitePreviewError("Choose a Finite site to preview.", 400);
  }
  const allowLocalOutputs = options.allowLocalOutputs ?? localOutputsEnabled();
  if (url.username || url.password || !allowedOutputHost(url, allowLocalOutputs)) {
    throw new SitePreviewError("Choose a Finite site to preview.", 400);
  }

  const returnTo = `${url.pathname}${url.search}${url.hash}`;
  if (
    !returnTo.startsWith("/") ||
    returnTo.startsWith("//") ||
    returnTo.length > MAX_RETURN_TO_BYTES ||
    returnTo.includes("\\") ||
    /[\u0000-\u0020\u007f]/u.test(returnTo)
  ) {
    throw new SitePreviewError("Choose a Finite site to preview.", 400);
  }

  return {
    outputUrl: `${url.origin}/`,
    returnTo,
    originalUrl: url.toString(),
  };
}

function allowedOutputHost(url: URL, allowLocalOutputs: boolean) {
  if (url.protocol === "https:" && !url.port) {
    return oneLabelUnder(url.hostname, "docs.finite.chat")
      || oneLabelUnder(url.hostname, "finite.chat");
  }
  if (allowLocalOutputs && url.protocol === "http:") {
    return oneLabelUnder(url.hostname, "docs.sites.localhost")
      || oneLabelUnder(url.hostname, "sites.localhost");
  }
  return false;
}

export function localOutputsEnabled(
  env: Record<string, string | undefined> = process.env
) {
  return env.NODE_ENV !== "production" && env.FC_SITES_ALLOW_LOCAL_OUTPUTS === "1";
}

function oneLabelUnder(hostname: string, baseDomain: string) {
  const suffix = `.${baseDomain}`;
  if (!hostname.endsWith(suffix)) return false;
  const label = hostname.slice(0, -suffix.length);
  return Boolean(label)
    && !label.includes(".")
    && label !== "api"
    && label !== "git";
}

export function parseViewerSessionResponse(payload: unknown, target: SitePreviewTarget) {
  if (!payload || typeof payload !== "object") {
    throw new SitePreviewError("Site previews aren't available right now.", 502);
  }
  const redeemUrl = (payload as { redeem_url?: unknown }).redeem_url;
  if (typeof redeemUrl !== "string" || redeemUrl.length > MAX_PREVIEW_URL_BYTES + 1024) {
    throw new SitePreviewError("Site previews aren't available right now.", 502);
  }

  let url: URL;
  try {
    url = new URL(redeemUrl);
  } catch {
    throw new SitePreviewError("Site previews aren't available right now.", 502);
  }
  const expectedOrigin = new URL(target.outputUrl).origin;
  const keys = Array.from(url.searchParams.keys()).sort();
  if (
    url.origin !== expectedOrigin ||
    url.pathname !== "/_finite/auth" ||
    url.username ||
    url.password ||
    url.hash ||
    keys.length !== 2 ||
    keys[0] !== "return_to" ||
    keys[1] !== "token" ||
    !/^[0-9a-f]{64}$/u.test(url.searchParams.get("token") ?? "") ||
    url.searchParams.get("return_to") !== target.returnTo
  ) {
    throw new SitePreviewError("Site previews aren't available right now.", 502);
  }
  return url.toString();
}
