import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";

import {
  ONE_TIME_KEY_WARNING,
  canAccessAdminOps,
  heartbeatAgeLabel,
  oneTimeKeyDisplay,
  oneTimeKeyError,
  type OneTimeKeyActionState,
} from "./admin-ops";

test("canAccessAdminOps only allows dashboard admins", () => {
  assert.equal(canAccessAdminOps({ isAdmin: true }), true);
  assert.equal(canAccessAdminOps({ isAdmin: false }), false);
  assert.equal(canAccessAdminOps(null), false);
  assert.equal(canAccessAdminOps(undefined), false);
});

test("heartbeatAgeLabel formats ages and degrades safely", () => {
  const now = Date.parse("2026-07-02T12:00:00Z");
  assert.equal(heartbeatAgeLabel(null, now), "never");
  assert.equal(heartbeatAgeLabel(undefined, now), "never");
  assert.equal(heartbeatAgeLabel("not-a-date", now), "unknown");
  assert.equal(heartbeatAgeLabel("2026-07-02T12:00:05Z", now), "just now");
  assert.equal(heartbeatAgeLabel("2026-07-02T11:59:30Z", now), "30s ago");
  assert.equal(heartbeatAgeLabel("2026-07-02T11:45:00Z", now), "15m ago");
  assert.equal(heartbeatAgeLabel("2026-07-02T02:00:00Z", now), "10h ago");
  assert.equal(heartbeatAgeLabel("2026-06-28T12:00:00Z", now), "4d ago");
});

test("oneTimeKeyDisplay only renders for a real issued key", () => {
  assert.equal(oneTimeKeyDisplay(null), null);
  assert.equal(oneTimeKeyDisplay(undefined), null);
  assert.equal(oneTimeKeyDisplay({ status: "idle" }), null);
  assert.equal(oneTimeKeyDisplay({ status: "error", error: "nope" }), null);
  assert.equal(
    oneTimeKeyDisplay({
      status: "issued",
      keyId: "fp_key_1",
      grantId: "fp_grant_1",
      rawKey: "   ",
      note: "",
    }),
    null,
  );

  const display = oneTimeKeyDisplay({
    status: "issued",
    keyId: "fp_key_1",
    grantId: "fp_grant_1",
    rawKey: " fpk_live_abc123 ",
    note: "",
  });
  assert.ok(display);
  assert.equal(display.keyId, "fp_key_1");
  assert.equal(display.grantId, "fp_grant_1");
  assert.equal(display.rawKey, "fpk_live_abc123");
  assert.equal(display.warning, ONE_TIME_KEY_WARNING);
});

test("oneTimeKeyDisplay keeps a Core-provided one-time note", () => {
  const display = oneTimeKeyDisplay({
    status: "issued",
    keyId: "fp_key_2",
    grantId: null,
    rawKey: "fpk_live_next",
    note: "This raw key is shown once.",
  });
  assert.ok(display);
  assert.equal(display.warning, "This raw key is shown once.");
});

test("oneTimeKeyError surfaces only error states", () => {
  assert.equal(oneTimeKeyError(null), null);
  assert.equal(oneTimeKeyError({ status: "idle" }), null);
  const issued: OneTimeKeyActionState = {
    status: "issued",
    keyId: "fp_key_1",
    grantId: null,
    rawKey: "fpk_live_x",
    note: "",
  };
  assert.equal(oneTimeKeyError(issued), null);
  assert.equal(oneTimeKeyError({ status: "error", error: "denied" }), "denied");
  assert.equal(
    oneTimeKeyError({ status: "error", error: "  " }),
    "The admin action failed.",
  );
});

test("runtime upgrade control stays admin-only and requires an exact artifact id", async () => {
  const [actionsSource, adminPageSource, upgradePageSource] = await Promise.all([
    readFile(path.resolve(process.cwd(), "src/app/actions.ts"), "utf8"),
    readFile(
      path.resolve(process.cwd(), "src/app/dashboard/admin/page.tsx"),
      "utf8"
    ),
    readFile(
      path.resolve(
        process.cwd(),
        "src/app/dashboard/admin/runtime-upgrade/page.tsx"
      ),
      "utf8"
    ),
  ]);

  assert.match(
    actionsSource,
    /adminOpsUpgradeRuntimeAction[\s\S]*requireAdminViewer\("upgrade hosted runtimes"\)[\s\S]*adminUpgradeCoreRuntime\([\s\S]*targetRuntimeArtifactId[\s\S]*redirect\("\/dashboard\/admin"\)/u
  );
  assert.doesNotMatch(adminPageSource, /name="targetRuntimeArtifactId"/u);
  assert.match(
    adminPageSource,
    /pathname: "\/dashboard\/admin\/runtime-upgrade"[\s\S]*query: \{ projectId: runtime\.project_id \}/u
  );
  assert.match(upgradePageSource, /canAccessAdminOps\(viewer\)/u);
  assert.match(upgradePageSource, /loadCoreAdminRuntimes\(\)/u);
  assert.match(
    upgradePageSource,
    /candidate\.project_id === projectId/u
  );
  assert.match(upgradePageSource, /name="targetRuntimeArtifactId"/u);
  assert.match(upgradePageSource, /required/u);
  assert.match(
    upgradePageSource,
    /No\s+candidate is selected automatically\./u
  );
});
