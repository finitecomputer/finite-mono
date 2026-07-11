import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  base: "./",
  plugins: [react()],
  // @finite/chat-ui is a source package linked from this consumer. Resolve
  // its React peers from this app exactly as an installed package would.
  resolve: {
    preserveSymlinks: true,
    dedupe: ["react", "react-dom"],
  },
  server: {
    port: 5179,
    strictPort: false,
  },
});
