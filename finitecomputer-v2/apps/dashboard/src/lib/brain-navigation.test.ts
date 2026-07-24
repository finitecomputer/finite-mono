import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("Brain navigation stays disabled while the direct testing route remains available", async () => {
  const source = await readFile(
    new URL("../components/agent-navigation.tsx", import.meta.url),
    "utf8",
  );
  const route = await readFile(
    new URL("../app/dashboard/machines/[machineId]/brain/page.tsx", import.meta.url),
    "utf8",
  );

  const item = source.match(/\{\s*label: "Brain",[\s\S]*?\n\s*\},/u)?.[0];
  assert.ok(item, "Brain navigation item is present");
  assert.doesNotMatch(item, /href:/u);
  assert.match(item, /active: false,/u);
  assert.match(item, /note: "Coming soon",/u);
  assert.match(route, /export default async function MachineBrainPage/u);
  assert.match(route, /<BrainFrame/u);
});
