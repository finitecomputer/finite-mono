import type { NextConfig } from "next";
import path from "node:path";

const tsconfigPath = process.env.NEXT_TSCONFIG_PATH?.trim();

// Spike: same-origin tunnel to a REMOTE hermes gateway. Gated gateways mint
// ws tickets behind cookie auth, and hermes' CORS allowlist has no
// credentials, so the browser needs a same-origin path. Opt-in via env:
//   HERMES_GATEWAY_PROXY_TARGET=https://host
//   NEXT_PUBLIC_HERMES_GATEWAY_WS_URL=ws://127.0.0.1:PORT/hermes-gateway/api/ws
// (plus NEXT_PUBLIC_HERMES_GATEWAY_USERNAME/PASSWORD for gated mode).
const gatewayProxyTarget = process.env.HERMES_GATEWAY_PROXY_TARGET?.trim();

const nextConfig: NextConfig = {
  distDir: process.env.NEXT_DIST_DIR?.trim() || ".next",
  output: "standalone",
  transpilePackages: ["@finite/chat-ui"],
  turbopack: {
    root: path.resolve(/* turbopackIgnore: true */ __dirname, "../../.."),
  },
  ...(tsconfigPath ? { typescript: { tsconfigPath } } : {}),
  ...(gatewayProxyTarget
    ? {
        rewrites: async () => [
          {
            source: "/hermes-gateway/:path*",
            destination: `${gatewayProxyTarget}/:path*`,
          },
        ],
      }
    : {}),
};

export default nextConfig;
