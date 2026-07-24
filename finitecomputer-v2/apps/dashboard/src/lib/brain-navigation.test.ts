import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("Brain navigation exposes the existing per-agent Brain route", async () => {
  const source = await readFile(
    new URL("../components/agent-navigation.tsx", import.meta.url),
    "utf8",
  );

  const item = source.match(/\{\s*label: "Brain",[\s\S]*?\n\s*\},/u)?.[0];
  assert.ok(item, "Brain navigation item is present");
  assert.match(item, /href: `\$\{root\}\/brain`,/u);
  assert.match(item, /active: pathname === `\$\{root\}\/brain`,/u);
  assert.doesNotMatch(item, /Temporarily unavailable/u);
});
