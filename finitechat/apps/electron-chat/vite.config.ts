import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const legacyDashboardSrc =
  process.env.FINITE_LEGACY_DASHBOARD_SRC
  ?? path.resolve(__dirname, "../../../finitecomputer/apps/dashboard/src");

export default defineConfig({
  base: "./",
  plugins: [react()],
  resolve: {
    alias: {
      "@": legacyDashboardSrc,
      "next/link": path.resolve(__dirname, "src/shims/next-link.tsx"),
    },
  },
  server: {
    port: 5179,
    strictPort: false,
  },
});
