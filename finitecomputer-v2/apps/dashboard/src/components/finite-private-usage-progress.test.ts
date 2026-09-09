import assert from "node:assert/strict";
import test from "node:test";

import {
  finitePrivateUsedPercent,
  formatWeightedTokens,
} from "./finite-private-usage-progress";

test("Finite Private progress clamps usage percentages safely", () => {
  assert.equal(finitePrivateUsedPercent(0, 100_000_000), 0);
  assert.equal(finitePrivateUsedPercent(75_000_000, 100_000_000), 75);
  assert.equal(finitePrivateUsedPercent(125_000_000, 100_000_000), 100);
  assert.equal(finitePrivateUsedPercent(-1, 100_000_000), 0);
  assert.equal(finitePrivateUsedPercent(1, 0), 0);
});

test("Finite Private progress formats exact weighted-token counts", () => {
  assert.equal(formatWeightedTokens(75_000_000), "75,000,000");
});
