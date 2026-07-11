import type { NextConfig } from "next";
import path from "node:path";

const nextConfig: NextConfig = {
  distDir: process.env.NEXT_DIST_DIR?.trim() || ".next",
  output: "standalone",
  transpilePackages: ["@finite/chat-ui"],
  webpack(config) {
    config.resolve.symlinks = false;
    return config;
  },
  turbopack: {
    root: path.resolve(/* turbopackIgnore: true */ __dirname, "../../.."),
    // @finite/chat-ui is a source-linked package outside this app directory.
    // Resolve its React peers from the consuming dashboard just as npm would
    // for a published package; Turbopack otherwise starts at the symlink's
    // real path and misses this app's node_modules.
    resolveAlias: {
      react: "./node_modules/react",
      "react-dom": "./node_modules/react-dom",
      "lucide-react": "./node_modules/lucide-react",
      "react-markdown": "./node_modules/react-markdown",
      "remark-gfm": "./node_modules/remark-gfm",
    },
  },
};

export default nextConfig;
