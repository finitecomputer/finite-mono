import { notFound, redirect } from "next/navigation";
import { wasmSpikeEnabled, WASM_SPIKE_CHAT_PATH } from "@/lib/wasm-spike";
export default function WasmSpikePage() {
  if (!wasmSpikeEnabled()) notFound();
  redirect(WASM_SPIKE_CHAT_PATH);
}
