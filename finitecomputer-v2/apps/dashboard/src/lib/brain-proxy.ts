import { NextRequest, NextResponse } from "next/server";

const REQUEST_HEADERS = [
  "accept",
  "authorization",
  "content-type",
  "if-modified-since",
  "if-none-match",
  "x-finitebrain-nostr",
  "x-nostr-authorization",
] as const;

const RESPONSE_HEADERS = [
  "cache-control",
  "content-disposition",
  "content-type",
  "etag",
  "last-modified",
] as const;

const MAX_BRAIN_REQUEST_BODY_BYTES = 1024 * 1024;
const BRAIN_PROXY_TIMEOUT_MS = 60_000;

class BrainRequestTooLargeError extends Error {}

export function brainUpstreamOrigin(value = process.env.FC_BRAIN_UPSTREAM_URL) {
  const candidate = value?.trim().replace(/\/$/u, "");
  if (!candidate) return null;

  try {
    const url = new URL(candidate);
    if ((url.protocol !== "http:" && url.protocol !== "https:") || url.pathname !== "/") {
      return null;
    }
    return url.origin;
  } catch {
    return null;
  }
}

export function brainProxyRequestHeaders(source: Pick<Headers, "get">) {
  const headers = new Headers();
  for (const name of REQUEST_HEADERS) {
    const value = source.get(name);
    if (value) headers.set(name, value);
  }
  return headers;
}

export async function proxyBrainRequest(
  request: NextRequest,
  prefix: "/client" | "/_admin" | "/health",
  path: string[] = [],
) {
  const baseUrl = brainUpstreamOrigin();
  if (!baseUrl) {
    return NextResponse.json({ error: "Brain isn't available right now." }, { status: 503 });
  }

  const suffix = path.map(encodeURIComponent).join("/");
  const upstream = new URL(suffix ? `${prefix}/${suffix}` : prefix, baseUrl);
  upstream.search = request.nextUrl.search;

  const headers = brainProxyRequestHeaders(request.headers);

  const controller = new AbortController();
  const abortForClient = () => controller.abort();
  request.signal.addEventListener("abort", abortForClient, { once: true });
  if (request.signal.aborted) controller.abort();
  const timeout = setTimeout(() => controller.abort(), BRAIN_PROXY_TIMEOUT_MS);
  const cleanup = () => {
    clearTimeout(timeout);
    request.signal.removeEventListener("abort", abortForClient);
  };
  try {
    const response = await fetch(upstream, {
      method: request.method,
      headers,
      body:
        request.method === "GET" || request.method === "HEAD"
          ? undefined
          : await readBoundedBrainRequestBody(
              request,
              MAX_BRAIN_REQUEST_BODY_BYTES,
              controller.signal,
            ),
      cache: "no-store",
      redirect: "manual",
      signal: controller.signal,
    });
    if (response.status >= 300 && response.status < 400) {
      await response.body?.cancel().catch(() => undefined);
      cleanup();
      return NextResponse.json(
        { error: "Brain returned an unexpected redirect." },
        { status: 502 },
      );
    }
    const responseHeaders = new Headers();
    for (const name of RESPONSE_HEADERS) {
      const value = response.headers.get(name);
      if (value) responseHeaders.set(name, value);
    }
    const noBody = request.method === "HEAD" || responseStatusHasNoBody(response.status);
    const body = noBody
      ? null
      : timedBrainResponseBody(response.body, controller, cleanup);
    if (noBody) cleanup();
    return new NextResponse(body, {
      status: response.status,
      headers: responseHeaders,
    });
  } catch (error) {
    cleanup();
    if (error instanceof BrainRequestTooLargeError) {
      return NextResponse.json({ error: "That Brain request is too large." }, { status: 413 });
    }
    if (controller.signal.aborted) {
      return NextResponse.json({ error: "Brain took too long to respond." }, { status: 504 });
    }
    return NextResponse.json({ error: "Brain isn't available right now." }, { status: 502 });
  }
}

function timedBrainResponseBody(
  body: ReadableStream<Uint8Array> | null,
  abortController: AbortController,
  cleanup: () => void,
) {
  if (!body) {
    cleanup();
    return null;
  }
  const reader = body.getReader();
  return new ReadableStream<Uint8Array>({
    async pull(controller) {
      try {
        const { done, value } = await reader.read();
        if (done) {
          cleanup();
          controller.close();
        } else {
          controller.enqueue(value);
        }
      } catch (error) {
        cleanup();
        controller.error(error);
      }
    },
    async cancel(reason) {
      cleanup();
      abortController.abort();
      await reader.cancel(reason);
    },
  });
}

export async function readBoundedBrainRequestBody(
  request: Request,
  maxBytes = MAX_BRAIN_REQUEST_BODY_BYTES,
  signal = request.signal,
) {
  const declaredLength = request.headers.get("content-length");
  if (declaredLength && (!/^\d+$/u.test(declaredLength) || Number(declaredLength) > maxBytes)) {
    throw new BrainRequestTooLargeError();
  }

  const reader = request.body?.getReader();
  if (!reader) return undefined;
  const chunks: Uint8Array[] = [];
  let length = 0;
  while (true) {
    const { done, value } = await readStreamChunk(reader, signal);
    if (done) break;
    length += value.byteLength;
    if (length > maxBytes) {
      await reader.cancel().catch(() => undefined);
      throw new BrainRequestTooLargeError();
    }
    chunks.push(value);
  }
  const body = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    body.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return body;
}

async function readStreamChunk(
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

export function responseStatusHasNoBody(status: number) {
  return status === 101 || status === 204 || status === 205 || status === 304;
}
