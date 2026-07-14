import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

const dashboardRoot = process.cwd();

test("public privacy and terms state the launch promises and billing contract", async () => {
  const policy = await readFile(path.join(dashboardRoot, "public/privacy.txt"), "utf8");
  const creationForm = await readFile(
    path.join(dashboardRoot, "src/components/core-agent-creation-form.tsx"),
    "utf8"
  );

  assert.match(policy, /We do not sell your data\./u);
  assert.match(policy, /We do not look at the contents of your private data during normal operation\./u);
  assert.match(policy, /\$200 USD per month/u);
  assert.match(policy, /renews automatically each month until you\s+cancel it/u);
  assert.match(policy, /cancellation takes effect at\s+the end of the current billing period/u);
  assert.match(creationForm, /href="\/privacy\.txt"/u);
});
