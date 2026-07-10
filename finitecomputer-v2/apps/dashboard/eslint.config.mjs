import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  // Override default ignores of eslint-config-next.
  globalIgnores([
    // Default ignores of eslint-config-next:
    ".next/**",
    // Browser integration tests use an isolated Next build directory so they
    // can run alongside the live local dashboard.
    ".next-browser-test/**",
    // Devfinity's long-lived dashboard is isolated from production and test
    // manifests for the same reason.
    ".next-devfinity/**",
    "out/**",
    "build/**",
    "next-env.d.ts",
  ]),
]);

export default eslintConfig;
