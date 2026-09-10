// Server-only local harness switch. Never available in a production build.
export function wasmSpikeEnabled() {
  return process.env.FINITECHAT_WASM_SPIKE === "1" && process.env.NODE_ENV !== "production";
}
export const WASM_SPIKE_MACHINE = "wasm-hermes";
export const WASM_SPIKE_CHAT_PATH = `/dashboard/machines/${WASM_SPIKE_MACHINE}/chat`;
