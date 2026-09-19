import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import test from "node:test";

// Evaluate a fresh module per environment, including accidental preview flags
// in production. The same guard owns both Brain and Sites sample data.
test("product samples require development, the preview flag, local auth, and the exact fixture agent", () => {
  for (const mode of ["production", "test", "development"] as const) {
    for (const preview of ["0", "1"]) {
      for (const localAuth of ["0", "1"]) {
        const result = execFileSync(process.execPath, ["--import", "tsx", "-e", `
          const { dashboardAgentDesignPreviewEnabled: enabled } = require("./src/lib/dashboard-design-preview.ts");
          console.log(JSON.stringify([enabled("runtime_web_design"), enabled("runtime_web_design_second")]));
        `], {
          encoding: "utf8",
          env: { ...process.env, NODE_ENV: mode, NEXT_PUBLIC_FC_DESIGN_PREVIEWS: preview, FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: localAuth },
        });
        assert.deepEqual(JSON.parse(result), [mode === "development" && preview === "1" && localAuth === "1", false], `${mode}, preview=${preview}, localAuth=${localAuth}`);
      }
    }
  }
});
