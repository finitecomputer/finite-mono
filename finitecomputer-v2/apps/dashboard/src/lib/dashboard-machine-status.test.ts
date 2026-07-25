import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { dashboardMachineStatusPresentation } from "@/lib/dashboard-machine-status";

const shellStylesUrl = new URL("../styles/ocean-shell.css", import.meta.url);

test("machine switcher presents every Core runtime status truthfully", () => {
  assert.deepEqual(dashboardMachineStatusPresentation("online"), {
    className: "is-online",
    label: "Online",
  });
  assert.deepEqual(dashboardMachineStatusPresentation("offline"), {
    className: "is-offline",
    label: "Offline",
  });
  assert.deepEqual(dashboardMachineStatusPresentation("stale"), {
    className: "is-stale",
    label: "Needs attention",
  });
  assert.deepEqual(dashboardMachineStatusPresentation("unknown"), {
    className: "is-unknown",
    label: "Status unknown",
  });
});

test("machine switcher uses red for offline and green only for online", async () => {
  const styles = await readFile(shellStylesUrl, "utf8");
  assert.match(
    styles,
    /\.ocean-machine-switcher__dot\.is-offline\s*\{[^}]*background:\s*var\(--danger-text\)/su
  );
  assert.match(
    styles,
    /\.ocean-machine-switcher__dot\.is-online\s*\{[^}]*background:\s*var\(--success\)/su
  );
});
