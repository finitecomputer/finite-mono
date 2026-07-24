import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("Brain navigation links to the selected agent instead of rendering disabled", async () => {
  const source = await readFile(
    new URL("../components/agent-navigation.tsx", import.meta.url),
    "utf8",
  );

  assert.match(source, /label: "Brain",[\s\S]*?href: `\$\{root\}\/brain`/u);
  assert.doesNotMatch(
    source,
    /label: "Brain",[\s\S]*?note: "Temporarily unavailable"/u,
  );
});
