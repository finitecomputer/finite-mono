import type { NextConfig } from "next";
import path from "node:path";

const nextConfig: NextConfig = {
  distDir: process.env.NEXT_DIST_DIR?.trim() || ".next",
  output: "standalone",
  transpilePackages: ["@finite/chat-ui"],
  turbopack: {
    root: path.resolve(/* turbopackIgnore: true */ __dirname, "../../.."),
  },
};

export default nextConfig;
