import { notFound } from "next/navigation";
import WasmSpike from "./wasm-spike";

export default function Page() {
  if (process.env.FINITECHAT_WASM_SPIKE !== "1" || process.env.NODE_ENV === "production") notFound();
  return <WasmSpike />;
}
