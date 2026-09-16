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

// Each status read gets a fresh peer and short-lived admission. No renewal loop,
// persistent browser keys or chat/session state are needed for a single read.
export async function readAgentStatus(projectId: string, signal: AbortSignal): Promise<unknown> {
  const budget = AbortSignal.any([signal, AbortSignal.timeout(30_000)]);
  let peer: Peer | undefined;
  let admission: { generation: number; endpointId: string; peerId: string } | undefined;
  let cancel: () => void = () => {};
  const cancelled = new Promise<never>((_, reject) => {
    cancel = () => {
      void peer?.close();
      reject(new Error("Status request cancelled or timed out"));
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
      // Runtime polls every five seconds. Retry transport setup for this read,
      // bounded by the single deadline; HTTP/application failures are final.
      while (true) {
        budget.throwIfAborted();
        let response: string;
        try {
          response = await peer.request(binding.endpointId, "GET", "/api/status", "{}", new Uint8Array());
        } catch {
          budget.throwIfAborted();
          await new Promise((resolve) => setTimeout(resolve, 1_000));
          continue;
        }
        budget.throwIfAborted();
        const result = JSON.parse(response) as { status: number; body: number[] };
        if (result.status !== 200) throw new Error(`Hermes returned HTTP ${result.status}`);
        return JSON.parse(new TextDecoder().decode(new Uint8Array(result.body)));
      }
    } finally {
      // This also closes a peer whose creation finishes after cancellation.
      if (peer) { const closing = peer; peer = undefined; await closing.close(); closing.free(); }
      if (admission) await transportControl(projectId, "DELETE", admission, AbortSignal.timeout(3_000)).catch(() => {});
    }
  };
  try { return await Promise.race([operation(), cancelled]); }
  finally { budget.removeEventListener("abort", cancel); }
}
