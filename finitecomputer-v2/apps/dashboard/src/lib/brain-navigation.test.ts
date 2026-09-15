import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("Brain prototype navigation is limited to the local design agent", async () => {
  const source = await readFile(
    new URL("../components/agent-navigation.tsx", import.meta.url),
    "utf8",
  );

  const item = source.match(/\{\s*label: "Brain",[\s\S]*?\n\s*\},/u)?.[0];
  assert.ok(item, "Brain navigation item is present");
  assert.match(source, /const brainPrototype = process\.env\.NODE_ENV === "development" && machineId === "runtime_web_design";/u);
  assert.match(item, /href: brainPrototype \? `\$\{root\}\/brain-prototype` : undefined,/u);
  assert.match(item, /active: brainPrototype &&/u);
  assert.match(item, /: "Coming soon",/u);
});
