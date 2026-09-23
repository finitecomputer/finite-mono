export type HostedHermesAccess = {
  nativeHermesChat?: boolean;
  runtimeId: string;
  enrolled: boolean;
  enabled: boolean;
  generation: number;
  appliedGeneration: number | null;
  applyStatus: "pending" | "applied" | "error";
};

export type HostedHermesSession = { baseUrl: string; accessToken: string; expiresAt: number };
export type NativeRequesterContext = { userId: string; expiresAt: number } &
  ({ email: string; sitesAssertion: string } | { email?: never; sitesAssertion?: never });

export async function readNativeRequesterContext(runtimeId: string, signal: AbortSignal): Promise<NativeRequesterContext | undefined> {
  return bounded(signal, async requestSignal => {
    const result = record(await controlRequest(runtimeId, {
      method: "POST", signal: requestSignal,
      headers: { "content-type": "application/json" }, body: JSON.stringify({ requester: true }),
    }));
    requestSignal.throwIfAborted();
    const value = record(result.requester);
    if (typeof value.userId !== "string" || !/^[0-9a-f]{64}$/.test(value.userId) ||
      !Number.isSafeInteger(value.expiresAt) || Number(value.expiresAt) <= 0) return undefined;
    const identity = { userId: value.userId, expiresAt: Number(value.expiresAt) };
    if (value.email === undefined && value.sitesAssertion === undefined) return identity;
    if (typeof value.email !== "string" || !value.email.includes("@") || value.email.length > 254 ||
      typeof value.sitesAssertion !== "string" || !/^[0-9a-f]{64}$/.test(value.sitesAssertion)) return undefined;
    return { ...identity, email: value.email, sitesAssertion: value.sitesAssertion };
  });
}

export type HostedHermesStatus = { version: string; gatewayRunning: boolean };
export class HostedHermesStatusError extends Error {
  constructor(message: string, readonly kind: "request" | "access" | "unsupported" = "request", readonly status?: number) {
    super(message);
  }
}

export function parseHostedHermesAccess(value: unknown, runtimeId: string): HostedHermesAccess {
  const access = record(value);
  if (
    access.runtimeId !== runtimeId || typeof access.enrolled !== "boolean" ||
    typeof access.enabled !== "boolean" || !generation(access.generation) ||
    !(access.appliedGeneration === null || generation(access.appliedGeneration)) ||
    !["pending", "applied", "error"].includes(String(access.applyStatus))
  ) throw new HostedHermesStatusError("Hosted access details are unavailable. Try again.");
  return {
    ...(typeof access.nativeHermesChat === "boolean" ? { nativeHermesChat: access.nativeHermesChat } : {}),
    runtimeId, enrolled: access.enrolled, enabled: access.enabled,
    generation: access.generation as number,
    appliedGeneration: access.appliedGeneration as number | null,
    applyStatus: access.applyStatus as HostedHermesAccess["applyStatus"],
  };
}

export function parseHostedHermesSession(value: unknown): HostedHermesSession {
  const session = record(value);
  const invalid = () => new HostedHermesStatusError("Hermes authorization is unavailable. Try again.");
  if (
    typeof session.baseUrl !== "string" || typeof session.accessToken !== "string" ||
    (!session.accessToken || session.accessToken.length > 8192) || typeof session.expiresAt !== "number" ||
    !Number.isSafeInteger(session.expiresAt) || session.expiresAt <= 0
  ) throw invalid();
  let url: URL;
  try { url = new URL(session.baseUrl); } catch { throw invalid(); }
  if (
    url.protocol !== "https:" || url.username || url.password || url.search || url.hash ||
    !url.pathname.endsWith("/")
  ) throw invalid();
  // Native/Core enforce lifetime. A user device clock is not an authority;
  // the operation-local request handles native expiry with one 401 retry.
  // Project only the intentionally browser-facing grant, never private native
  // password/refresh credentials or deployment metadata returned in the future.
  return { baseUrl: url.href, accessToken: session.accessToken, expiresAt: session.expiresAt };
}

export function parseHostedHermesStatus(value: unknown): HostedHermesStatus {
  const status = record(value);
  if (
    typeof status.version !== "string" || !status.version.trim() || status.version.length > 128 ||
    typeof status.gateway_running !== "boolean"
  ) throw new HostedHermesStatusError("Hermes returned an unexpected status response.");
  return { version: status.version, gatewayRunning: status.gateway_running };
}

export function hostedHermesAccessApplied(access: HostedHermesAccess) {
  return access.applyStatus === "applied" && access.appliedGeneration === access.generation;
}

export async function readHostedHermesAccess(runtimeId: string, signal: AbortSignal) {
  return bounded(signal, async (requestSignal) => parseHostedHermesAccess(
    await controlRequest(runtimeId, { signal: requestSignal }), runtimeId
  ));
}

export async function changeHostedHermesAccess(access: HostedHermesAccess, enabled: boolean, signal: AbortSignal) {
  return bounded(signal, async (requestSignal) => parseHostedHermesAccess(
    await controlRequest(access.runtimeId, {
      method: "PUT", signal: requestSignal,
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ enabled, expectedGeneration: access.generation }),
    }), access.runtimeId
  ));
}

/** One operation, one account-authorized grant. Credentials live only in this
 * call. A caller changing accounts/agents aborts its signal; no shared browser
 * token cache can carry authorization across that switch. */
export async function readHostedHermesJson(runtimeId: string, path: string, signal: AbortSignal): Promise<unknown> {
  return hostedHermesJson(runtimeId, path, signal);
}

export function setHostedHermesSessionArchived(runtimeId: string, sessionId: string, archived: boolean, signal: AbortSignal) {
  return hostedHermesJson(runtimeId, `api/sessions/${encodeURIComponent(sessionId)}`, signal, { archived });
}

async function hostedHermesJson(runtimeId: string, path: string, signal: AbortSignal, patch?: { archived: boolean }): Promise<unknown> {
  if (path !== "api/skills?inventory=true" && !/^api\/sessions\?limit=100&archived=include&order=created&offset=\d+$/.test(path) && (!/^api\/[a-zA-Z0-9_/-]+$/.test(path) || path.includes("//"))) {
    throw new HostedHermesStatusError("Invalid agent API path.");
  }
  return bounded(signal, async (requestSignal) => {
    for (let attempt = 0; attempt < 2; attempt++) {
      const grant = parseHostedHermesSession(await controlRequest(runtimeId, {
        method: "POST", signal: requestSignal,
      }));
      requestSignal.throwIfAborted();
      const response = await fetch(new URL(path, grant.baseUrl), {
        credentials: "omit", cache: "no-store", redirect: "error",
        referrerPolicy: "no-referrer", signal: requestSignal,
        method: patch ? "PATCH" : "GET",
        headers: { authorization: `Bearer ${grant.accessToken}`, ...(patch ? { "content-type": "application/json" } : {}) },
        ...(patch ? { body: JSON.stringify(patch) } : {}),
      });
      // A read may race native expiry/restart. Reauthorize once; never retry a
      // mutation or expose a native login form. Other failures remain visible.
      if (!patch && response.status === 401 && attempt === 0) {
        await response.body?.cancel();
        continue;
      }
      if (!response.ok) {
        await response.body?.cancel();
        if ([401, 403].includes(response.status)) {
          throw new HostedHermesStatusError("Access to this agent is no longer available.", "access");
        }
        if (response.status === 404) {
          throw new HostedHermesStatusError("This agent does not support this page yet.", "unsupported");
        }
        throw new HostedHermesStatusError("Agent access is unavailable. Try again.");
      }
      const result = await boundedJson(response, "unsupported");
      requestSignal.throwIfAborted();
      return result;
    }
    throw new HostedHermesStatusError("Agent authorization is unavailable. Try again.");
  });
}

/** Every reconnect gets a new owner-authorized, single-use native ticket. */
export async function createHostedHermesWebSocket(runtimeId: string, signal: AbortSignal): Promise<{ url: string; protocols: string[] }> {
  return bounded(signal, async (requestSignal) => {
    const grant = parseHostedHermesSession(await controlRequest(runtimeId, {
      method: "POST", signal: requestSignal,
    }));
    requestSignal.throwIfAborted();
    const response = await fetch(new URL("api/auth/ws-ticket", grant.baseUrl), {
      method: "POST", credentials: "omit", cache: "no-store", redirect: "error",
      referrerPolicy: "no-referrer", signal: requestSignal,
      headers: { authorization: `Bearer ${grant.accessToken}` },
    });
    if (!response.ok) {
      await response.body?.cancel();
      throw new HostedHermesStatusError("Agent chat authorization is unavailable.");
    }
    const result = await boundedJson(response);
    if (!result || typeof result !== "object" || !("ticket" in result)
      || typeof result.ticket !== "string" || !result.ticket || result.ticket.length > 4096) {
      throw new HostedHermesStatusError("Agent returned an invalid chat ticket.");
    }
    requestSignal.throwIfAborted();
    const url = new URL("api/ws", grant.baseUrl);
    url.protocol = "wss:";
    return { url: url.toString(), protocols: ["hermes-gateway-v1", `hermes-gateway-ticket.${result.ticket}`] };
  });
}

export async function readHostedHermesStatus(runtimeId: string, signal: AbortSignal): Promise<HostedHermesStatus> {
  return parseHostedHermesStatus(await readHostedHermesJson(runtimeId, "api/status", signal));
}

async function boundedJson(response: Response, invalidKind: "request" | "unsupported" = "request"): Promise<unknown> {
  const reader = response.body?.getReader();
  if (!reader) throw new HostedHermesStatusError("Agent returned an empty response.", invalidKind);
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > 1024 * 1024) throw new HostedHermesStatusError("Agent response is too large.", invalidKind);
      chunks.push(value);
    }
    const data = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) { data.set(chunk, offset); offset += chunk.length; }
    try { return JSON.parse(new TextDecoder().decode(data)); }
    catch { throw new HostedHermesStatusError("Agent returned an incompatible response.", invalidKind); }
  } finally {
    await reader.cancel();
    reader.releaseLock();
  }
}

async function controlRequest(runtimeId: string, init: RequestInit): Promise<unknown> {
  const response = await fetch(`/api/agents/${encodeURIComponent(runtimeId)}/hermes-access`, {
    ...init, credentials: "same-origin", cache: "no-store", redirect: "error",
  });
  if (!response.ok) {
    const message = response.status === 409
      ? "This agent’s dashboard connection is not ready yet. Try again shortly."
      : [401, 403, 404].includes(response.status)
        ? "Hosted access is unavailable for this account or agent."
        : "The agent’s dashboard connection is unavailable. Try again.";
    await response.body?.cancel();
    throw new HostedHermesStatusError(message, [401, 403, 404, 409].includes(response.status) ? "access" : "request", response.status);
  }
  return boundedJson(response);
}

async function bounded<T>(signal: AbortSignal, action: (signal: AbortSignal) => Promise<T>): Promise<T> {
  const deadline = AbortSignal.timeout(15_000);
  const requestSignal = AbortSignal.any([signal, deadline]);
  try {
    requestSignal.throwIfAborted();
    return await action(requestSignal);
  } catch (error) {
    if (deadline.aborted) throw new HostedHermesStatusError("The request timed out. Try again.");
    if (signal.aborted) throw error;
    if (error instanceof HostedHermesStatusError) throw error;
    throw new HostedHermesStatusError("Could not reach Hermes from this browser. Try again.");
  }
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? value as Record<string, unknown> : {};
}

function generation(value: unknown) {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
