/** Local gateway UI with the same hot-reloading dashboard fixture as design. */
import fs from "node:fs";
import { spawn } from "node:child_process";

const configPath = ".env.gateway.local";
if (fs.existsSync(configPath)) process.loadEnvFile(configPath);
const gateway = process.env.HERMES_GATEWAY_WS_URL?.trim();
if (!gateway) throw new Error("Set HERMES_GATEWAY_WS_URL in .env.gateway.local (see docs/gateway-chat.md).");
const url = new URL(gateway);
if (!["ws:", "wss:"].includes(url.protocol)) throw new Error("Gateway URL must use ws:// or wss://.");
if (url.username || url.password || url.search || url.hash) {
  throw new Error("Use a gateway URL without credentials or query parameters; set HERMES_GATEWAY_TOKEN separately.");
}
const child = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
  stdio: "inherit",
  env: {
    ...process.env,
    FC_WEB_DESIGN_PORT: process.env.FC_WEB_DESIGN_PORT || "13485",
    NEXT_PUBLIC_HERMES_GATEWAY_WS_URL: gateway,
    NEXT_PUBLIC_HERMES_GATEWAY_PUBLIC_URL: gateway,
    NEXT_PUBLIC_HERMES_GATEWAY_TOKEN: process.env.HERMES_GATEWAY_TOKEN || "",
    NEXT_PUBLIC_HERMES_GATEWAY_USERNAME: process.env.HERMES_GATEWAY_USERNAME || "",
    NEXT_PUBLIC_HERMES_GATEWAY_PASSWORD: process.env.HERMES_GATEWAY_PASSWORD || "",
  },
});
for (const signal of ["SIGINT", "SIGTERM"] as const) process.on(signal, () => child.kill(signal));
child.on("exit", (code) => { process.exitCode = code ?? 1; });
console.log(`Gateway chat: http://127.0.0.1:${process.env.FC_WEB_DESIGN_PORT || "13485"}/dashboard/machines/runtime_web_design/gateway-chat`);
