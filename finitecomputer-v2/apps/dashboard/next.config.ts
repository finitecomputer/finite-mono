import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  distDir: process.env.NEXT_DIST_DIR?.trim() || ".next",
  output: "standalone",
  async rewrites() {
    const brain = trustedHttpOrigin(process.env.FC_BRAIN_UPSTREAM_URL);
    if (!brain) return [];
    return [
      { source: "/client", destination: `${brain}/client` },
      { source: "/client/:path*", destination: `${brain}/client/:path*` },
      { source: "/_admin/:path*", destination: `${brain}/_admin/:path*` },
    ];
  },
};

function trustedHttpOrigin(value: string | undefined) {
  const candidate = value?.trim().replace(/\/$/u, "");
  if (!candidate) return null;
  try {
    const url = new URL(candidate);
    if ((url.protocol !== "http:" && url.protocol !== "https:") || url.pathname !== "/") {
      return null;
    }
    return url.origin;
  } catch {
    return null;
  }
}

export default nextConfig;
