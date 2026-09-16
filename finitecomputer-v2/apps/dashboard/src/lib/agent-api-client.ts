import type { AgentEndpoint } from "./core-client";

type Peer = {
  id(): string;
  request(agent: string, method: string, path: string, headers: string, body: Uint8Array): Promise<string>;
  close(): Promise<void>;
  free(): void;
};
type BrowserModule = {
  default(options: { module_or_path: Response }): Promise<unknown>;
  BrowserPeer: { create(relay: string): Promise<Peer> };
};

export async function transportControl(projectId: string, method = "GET", payload?: unknown, signal = AbortSignal.timeout(15_000)) {
  const response = await fetch(`/api/agents/${encodeURIComponent(projectId)}/transport`, {
    method, cache: "no-store", signal,
    ...(payload === undefined ? {} : {
      headers: { "content-type": "application/json" }, body: JSON.stringify(payload),
    }),
  });
  const data = await response.json();
  if (!response.ok) throw new Error(data.error || "Agent access unavailable");
  return data;
}

// Each API read gets a fresh peer and short-lived admission. No renewal loop,
// persistent browser keys or chat/session state are needed for a single read.
/** Read-only native Hermes API, authenticated through its loopback bootstrap.
 * Tokens live only for this operation, never in Core, cookies or local storage.
 */
export async function readAgentJson(projectId: string, path: string, signal: AbortSignal): Promise<unknown> {
  if (!path.startsWith("/api/") || /[\r\n#]/.test(path)) throw new Error("Invalid agent API path");
  const budget = AbortSignal.any([signal, AbortSignal.timeout(30_000)]);
  let peer: Peer | undefined;
  let admission: { generation: number; endpointId: string; peerId: string } | undefined;
  let cancel: () => void = () => {};
  const cancelled = new Promise<never>((_, reject) => {
    cancel = () => {
      void peer?.close();
      reject(new Error("Agent request cancelled or timed out"));
    };
    budget.addEventListener("abort", cancel, { once: true });
    if (budget.aborted) cancel();
  });
  const operation = async () => {
    try {
      budget.throwIfAborted();
      const binding: AgentEndpoint = await transportControl(projectId, "GET", undefined, budget);
      if (!binding.enabled) throw new Error("Hosted access is disabled");
      const moduleUrl = "/iroh/client.js";
      const wasm: BrowserModule = await import(/* webpackIgnore: true */ /* turbopackIgnore: true */ moduleUrl);
      budget.throwIfAborted();
      const binary = await fetch("/iroh/client_bg.wasm", { signal: budget });
      if (!binary.ok) throw new Error("Browser transport is unavailable");
      await wasm.default({ module_or_path: binary });
      budget.throwIfAborted();
      peer = await wasm.BrowserPeer.create(binding.relayUrl);
      budget.throwIfAborted();
      admission = { generation: binding.generation, endpointId: binding.endpointId, peerId: peer.id() };
      await transportControl(projectId, "POST", admission, budget);
      // Retry setup while the runtime's five-second admission poll catches up.
      // HTTP/application errors stay visible and do not trigger transport retries.
      const get = async (target: string, token?: string) => {
        while (true) {
          budget.throwIfAborted();
          let response: string;
          try {
            response = await peer!.request(binding.endpointId, "GET", target,
              JSON.stringify(token ? { "X-Hermes-Session-Token": token } : {}), new Uint8Array());
          } catch {
            budget.throwIfAborted();
            await new Promise((resolve) => setTimeout(resolve, 1_000));
            continue;
          }
          budget.throwIfAborted();
          const result = JSON.parse(response) as { status: number; body: number[] };
          return { status: result.status, text: new TextDecoder().decode(new Uint8Array(result.body)) };
        }
      };
      const bootstrap = async () => {
        const response = await get("/");
        if (response.status !== 200) throw new Error("Native Hermes bootstrap unavailable");
        return parseHermesBootstrap(response.text);
      };
      let token = await bootstrap();
      let result = await get(path, token);
      // Hermes may restart between bootstrap and the protected read. Refresh
      // once on 401; never retry arbitrary failures or switch authentication mode.
      if (result.status === 401) {
        token = await bootstrap();
        result = await get(path, token);
      }
      if (result.status !== 200) throw new Error(`Hermes returned HTTP ${result.status}`);
      return JSON.parse(result.text);
    } finally {
      // This also closes a peer whose creation finishes after cancellation.
      if (peer) { const closing = peer; peer = undefined; await closing.close(); closing.free(); }
      if (admission) await transportControl(projectId, "DELETE", admission, AbortSignal.timeout(3_000)).catch(() => {});
    }
  };
  try { return await Promise.race([operation(), cancelled]); }
  finally { budget.removeEventListener("abort", cancel); }
}


export function parseHermesBootstrap(html: string): string {
  // Parse the native JSON assignment as data; never execute returned HTML/JS.
  if (!/window\.__HERMES_AUTH_REQUIRED__\s*=\s*false\s*;/.test(html)) {
    throw new Error("Hermes requires a different authentication mode");
  }
  const match = html.match(/window\.__HERMES_SESSION_TOKEN__\s*=\s*("(?:[^"\\]|\\.)*")\s*;/);
  if (!match) throw new Error("Native Hermes bootstrap unavailable");
  const token: unknown = JSON.parse(match[1]);
  if (typeof token !== "string" || !/^[A-Za-z0-9_-]{16,512}$/.test(token)) {
    throw new Error("Invalid native Hermes bootstrap");
  }
  return token;
}

export function readAgentStatus(projectId: string, signal: AbortSignal) {
  return readAgentJson(projectId, "/api/status", signal);
}

export type AgentSkill = {
  name: string;
  description: string;
  category?: string | null;
  enabled: boolean;
  provenance: "agent" | "bundled" | "hub";
  usage: number;
};

export async function readAgentSkills(projectId: string, signal: AbortSignal): Promise<AgentSkill[]> {
  const value = await readAgentJson(projectId, "/api/skills", signal);
  if (!Array.isArray(value) || !value.every((skill) => skill &&
    typeof skill.name === "string" && typeof skill.description === "string" &&
    typeof skill.enabled === "boolean" && typeof skill.usage === "number" &&
    ["agent", "bundled", "hub"].includes(skill.provenance))) {
    throw new Error("Hermes returned an unexpected skills list");
  }
  return value;
}
